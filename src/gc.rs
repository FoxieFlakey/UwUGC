use std::{
    io::{LineWriter, stdout},
    sync::{Arc, atomic::Ordering},
};

use crate::{
    gc::relocation_map::{FrozenRegistry, RegistryBuilder, RelocationRecord},
    gc_controller::GCController,
    gc_sync::GCSync,
    mm::{BASE_PAGE_SIZE, Context, PageTable},
    mmap::Mmap,
    object::{MetadataCompressed, ObjectPtr},
    profiler::{Profiler, SectionCookie},
    root_set::RootSet,
    state::SharedState,
};

mod relocation_map;

pub struct PersistentState {
    cached_page_table: Option<PageTable>,
    cached_temp_mapping: Option<Mmap>,
    cached_registry: Option<RegistryBuilder>,
    args: GCArgs,
}

#[derive(Clone)]
pub struct GCArgs {
    pub preferred_temp_base: Option<usize>,
}

pub fn do_cycle(shared: &Arc<GCSync<SharedState>>, controller: &Arc<GCController>, args: &GCArgs) {
    let mut profiler = Profiler::new();
    profiler.start(|scope| {
        let ret = scope.section("(Conc) Init", |scope| init(scope, shared, controller, args));
        let ret = scope.section("(STW ) Step 1", |scope| step1(scope, ret));
        let ret = scope.section("(Conc) Step 2", |scope| step2(scope, ret));
        let ret = scope.section("(STW ) Step 3", |scope| step3(scope, ret));
        let ret = scope.section("(Conc) Step 4", |scope| step4(scope, ret));
        scope.section("(STW ) Step 5", |scope| step5(scope, ret));
    });

    profiler.report(&mut LineWriter::new(stdout()));
}

pub struct CommonArgs<'a> {
    shared: &'a Arc<GCSync<SharedState>>,
    controller: &'a Arc<GCController>,
    gc: PersistentState,
}

/// Step 0: (Concurrent) Initialize cycle
/// (its STW because PersistentState should not be in SharedState)
pub fn init<'a>(
    _section_cookie: &mut SectionCookie,
    shared: &'a Arc<GCSync<SharedState>>,
    controller: &'a Arc<GCController>,
    args: &GCArgs,
) -> Step1Args<'a> {
    let mut heap = shared.get_exclusive();
    let len = heap.get().mm.get_mapping().len();
    let heap_base = heap.get().mm.get_mapping().get_ptr();
    let nr_pages = heap.get().mm.get_page_table().nr_pages();
    let gc = heap
        .get()
        .gc_state
        .take()
        .unwrap_or_else(|| {
            // GC may store persistent state like caching few stuffs
            // or store reusable stuffs to avoid reallocating on each cycle
            PersistentState {
                cached_page_table: Some(PageTable::new(heap_base, nr_pages)),
                cached_temp_mapping: Some(Mmap::map(len, true, true, true, args.preferred_temp_base).unwrap()),
                cached_registry: Some(RegistryBuilder::new()),
                args: args.clone()
            }
        });

    Step1Args {
        common: CommonArgs {
            controller,
            shared,
            gc,
        },
    }
}

pub struct Step1Args<'a> {
    common: CommonArgs<'a>,
}

/// Step 1: (STW) Take snapshot of root, and capture some heap states
pub fn step1<'a>(section_cookie: &mut SectionCookie, args: Step1Args<'a>) -> Step2Args<'a> {
    let mut heap = section_cookie.section("STW wait", |_| args.common.shared.get_exclusive());

    // Take snapshot of root set
    let saved_roots = heap
        .get()
        .contexts
        .get_mut()
        .iter()
        .map(|(_, v)| {
            let mut guard = v.lock();
            let root_set = &mut guard.root_set;

            let raw_cloned = root_set.get_raw_mut().clone();

            root_set.clone_metadata(raw_cloned)
        })
        .collect::<Vec<_>>();

    // Ignore any request that happen during or before root snapshot
    // and after GC started
    args.common.controller.clear_request();

    Step2Args {
        common: args.common,
        heap_start: heap.get().mm.get_mapping().get_ptr(),
        used_end: heap.get().mm.get_page_table().get_top_addr() as *mut u8,
        root: saved_roots,
        heap_size: heap.get().mm.get_mapping().len(),
    }
}

pub struct Step2Args<'a> {
    common: CommonArgs<'a>,
    heap_start: *mut u8,
    used_end: *mut u8,
    root: Vec<Box<dyn RootSet>>,
    heap_size: usize,
}

/// Step 2: Perform concurrent marking using saved root
pub fn step2<'a>(_section_cookie: &mut SectionCookie, mut args: Step2Args<'a>) -> Step3Args<'a> {
    let mut registry = args.common.gc.cached_registry.take().unwrap();
    let mut move_context = Context::new();
    let page_table = args
        .common
        .gc
        .cached_page_table
        .take()
        .map(|mut x| {
            x.set_base(args.heap_start);
            x
        })
        .unwrap_or_else(|| PageTable::new(args.heap_start, (args.heap_size) / BASE_PAGE_SIZE));

    // Mark objects concurrently, note for now the mark bit doesnt
    // get used its assume all dead
    let mut live_count = 0;
    let mut total_count = 0;
    let mut visitor = |obj: &ObjectPtr| {
        // Mark the object
        let ret = obj
            .metadata_ref()
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |mut x| {
                if x.is_marked {
                    None
                } else {
                    x.is_marked = true;
                    Some(x)
                }
            });

        total_count += 1;
        if ret.is_ok() {
            // This just first marked.
            // TODO: push object to mark stack to be continued
            // recusrively
            live_count += 1;

            let size = obj.size() + size_of::<MetadataCompressed>();

            // SAFETY: We use same page table consistently
            let dest = unsafe { move_context.alloc_from_page_table(&page_table, size) }
                .unwrap()
                .0
                .addr();
            registry.insert(RelocationRecord {
                src: obj.to_ptr().addr(),
                dest,
                size,
            });
        } else {
            // Already marked this, either a while ago, or another thread
        }
    };

    for root in args.root {
        root.iter_pointers(&mut visitor);
    }

    let used = args.used_end.addr() - args.heap_start.addr();
    let compacted = page_table.get_top_addr() - args.heap_start.addr();
    println!("[GC] Live count: {:9}", live_count);
    println!("[GC] Compacted from {:#16} to {:#16}", bytesize::mib(u64::try_from(used).unwrap()), bytesize::mib(u64::try_from(compacted).unwrap()));

    Step3Args {
        common: args.common,
        page_table,
        relocation_registry: registry,
    }
}

pub struct Step3Args<'a> {
    common: CommonArgs<'a>,
    page_table: PageTable,
    relocation_registry: RegistryBuilder,
}

/// Step 3: (STW) Prepare for relocation and fix root pointer
pub fn step3<'a>(section_cookie: &mut SectionCookie, mut args: Step3Args<'a>) -> Step4Args<'a> {
    let mut heap = section_cookie.section("STW wait", |_| args.common.shared.get_exclusive());
    let registry_frozen = args.relocation_registry.freeze();

    // SAFETY: For now, we assume all objects are dead
    let mut page_table_opt = Some(args.page_table);
    let (mut page_table, mapping) = unsafe {
        section_cookie.section("Remap heap", |_| {
            heap.get()
                .mm
                .remap(&mut args.common.gc.cached_temp_mapping, &mut page_table_opt)
        })
    }
    .unwrap();
    assert!(
        page_table_opt.is_none(),
        "Expecting remap used the page table"
    );

    if let Some(preferred_temp_base) = args.common.gc.args.preferred_temp_base {
        assert_eq!(mapping.get_ptr().addr(), preferred_temp_base, "kernel moved the remap target! should have been 0x{preferred_temp_base:16} but moved to 0x{:16}", mapping.get_ptr().addr());
    }

    section_cookie.section("Fix root", |_| {
        heap.get().contexts.get_mut().iter().for_each(|x| {
            let mut root_set = x.1.lock();
            let root_set = &mut root_set.root_set;

            root_set.map_pointers(&mut |x| {
                let record = registry_frozen
                    .map_src_to_dest(x.to_ptr().addr())
                    .expect("Cannot find relocation record");
                // SAFETY: This points to correct address after relocated
                unsafe { ObjectPtr::new(record as *mut u8) }
            });
        });
    });

    page_table.clear();
    args.common.gc.cached_page_table = Some(page_table);
    args.common.gc.cached_temp_mapping = Some(mapping);
    Step4Args {
        common: args.common,
        relocation_registry: registry_frozen,
    }
}

pub struct Step4Args<'a> {
    common: CommonArgs<'a>,
    relocation_registry: FrozenRegistry,
}

/// Step 4: (Concurrent) Relocate
pub fn step4<'a>(_section_cookie: &mut SectionCookie, mut args: Step4Args<'a>) -> Step5Args<'a> {
    args.common.gc.cached_registry = Some(args.relocation_registry.unfreeze());

    // nothing, because actual relocation is not implemented yet
    Step5Args {
        common: args.common,
    }
}

pub struct Step5Args<'a> {
    common: CommonArgs<'a>,
}

/// Step 5: (STW) Finalize cycle
pub fn step5(section_cookie: &mut SectionCookie, args: Step5Args<'_>) {
    let mut heap = section_cookie.section("STW wait", |_| args.common.shared.get_exclusive());
    heap.get()
        .gc_state
        .set(args.common.gc)
        .ok()
        .expect("GC persistent state somehow is initialized?");
}
