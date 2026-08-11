use std::{
    io::{LineWriter, stdout},
    mem,
    sync::Arc,
};

use crate::{
    gc::{copier::CopierActive, marker::Marker, relocation_map::RegistryBuilder},
    gc_controller::GCController,
    gc_sync::GCSync,
    mm::MM,
    object::{Bit, ObjectPtr},
    profiler::{Profiler, SectionCookie},
    root_set::RootSet,
    state::SharedState,
};

mod copier;
mod marker;
mod relocation_map;

pub struct PersistentState {
    cached_registry: Option<RegistryBuilder>,
    cached_marker: Option<Marker>,
    #[expect(unused)]
    args: GCArgs,
    copier: Option<copier::Copier>,
}

#[derive(Clone)]
pub struct GCArgs {}

pub fn do_cycle(
    shared: &Arc<GCSync<SharedState>>,
    controller: &Arc<GCController>,
    args: &GCArgs,
    state: Option<PersistentState>,
) -> PersistentState {
    let mut profiler = Profiler::new();
    let ret = profiler.start(|scope| {
        let ret = scope.section("(Conc) Init", |scope| {
            init(scope, shared, controller, args, state)
        });
        let ret = scope.section("(STW ) Step 1", |scope| step1(scope, ret));
        let ret = scope.section("(Conc) Step 2", |scope| step2(scope, ret));
        let ret = scope.section("(STW ) Step 3", |scope| step3(scope, ret));
        let ret = scope.section("(Conc) Step 4", |scope| step4(scope, ret));
        scope.section("(STW ) Step 5", |scope| step5(scope, ret))
    });

    profiler.report(&mut LineWriter::new(stdout()));

    ret
}

pub struct CommonArgs<'a> {
    shared: &'a Arc<GCSync<SharedState>>,
    controller: &'a Arc<GCController>,
    gc: PersistentState,
}

/// Step 0: (Concurrent) Initialize cycle
fn init<'a>(
    _section_cookie: &mut SectionCookie,
    shared: &'a Arc<GCSync<SharedState>>,
    controller: &'a Arc<GCController>,
    args: &GCArgs,
    persisent_state: Option<PersistentState>,
) -> Step1Args<'a> {
    let gc = persisent_state.unwrap_or_else(|| {
        // GC may store persistent state like caching few stuffs
        // or store reusable stuffs to avoid reallocating on each cycle
        PersistentState {
            cached_registry: Some(RegistryBuilder::new()),
            cached_marker: Some(Marker::new()),
            args: args.clone(),
            copier: Some(copier::Copier::new()),
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

struct Step1Args<'a> {
    common: CommonArgs<'a>,
}

/// Step 1: (STW) Take snapshot of root, and capture some heap states
fn step1<'a>(section_cookie: &mut SectionCookie, args: Step1Args<'a>) -> Step2Args<'a> {
    let mut heap = section_cookie.section("STW wait", |_| args.common.shared.get_exclusive());
    let mark_true_bit = heap.get().mark_true_bit;

    // Flip the bit meaning
    heap.get().mark_true_bit = !mark_true_bit;

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

    // Lets assume all types are dead
    heap.get().type_manager.type_manager.assume_all_dead();

    let second_mm = heap.get().second_mm.take().unwrap();
    Step2Args {
        common: args.common,
        root: saved_roots,
        heap: HeapInfo {
            start: heap.get().mm.get_mapping().get_ptr(),
            used_end: heap.get().mm.get_page_table().get_top_addr() as *mut u8,
            size: heap.get().mm.get_mapping().len(),
            used_end_page: heap.get().mm.get_page_table().get_used_end_page(),
            to_space: second_mm.get_mapping().get_ptr(),
        },
        mark_true_bit,
        second_mm: second_mm,
    }
}

struct HeapInfo {
    start: *mut u8,
    used_end: *mut u8,
    size: usize,
    used_end_page: usize,
    to_space: *mut u8,
}

#[expect(unused)]
#[derive(Clone)]
struct HeapInfoLater {
    start: *mut u8,
    used_end: *mut u8,
    to_space: *mut u8,
    size: usize,
    used_end_page: usize,
    compacted_end: *mut u8,
    compacted_end_page: usize,

    // During concurrent phase 2. there may be new objects added, which need compacted back
    // used_end..later_used_end would be range of where new objects added. Which must be
    // left alone or unrelocated
    later_used_end: *mut u8,
    later_used_end_page: usize,
}

struct Step2Args<'a> {
    common: CommonArgs<'a>,
    root: Vec<Box<dyn RootSet>>,
    heap: HeapInfo,
    mark_true_bit: Bit,
    second_mm: MM,
}

/// Step 2: Perform concurrent marking using saved root
fn step2<'a>(_section_cookie: &mut SectionCookie, mut args: Step2Args<'a>) -> Step3Args<'a> {
    let registry = args.common.gc.cached_registry.take().unwrap();
    let page_table = args.second_mm.get_page_table_mut();

    let heap_guard = args.common.shared.get_shared();
    let (marker, registry) = args.common.gc.cached_marker.take().unwrap().start(
        args.root,
        page_table,
        registry,
        heap_guard.get(),
        &args.heap,
        args.mark_true_bit,
    );

    args.common.gc.cached_marker = Some(marker);
    Step3Args {
        common: args.common,
        compacted_end: page_table.get_top_addr() as *mut u8,
        second_mm: args.second_mm,
        relocation_registry: registry,
        heap: args.heap,
    }
}

struct Step3Args<'a> {
    common: CommonArgs<'a>,
    second_mm: MM,
    relocation_registry: RegistryBuilder,
    heap: HeapInfo,
    compacted_end: *mut u8,
}

/// Step 3: (STW) Prepare for relocation and fix root pointer
fn step3<'a>(section_cookie: &mut SectionCookie, mut args: Step3Args<'a>) -> Step4Args<'a> {
    let mut heap = section_cookie.section("STW wait", |_| args.common.shared.get_exclusive());
    // This has to be retrieved before remap, as remap replaces
    // page table
    let later_used_end = heap.get().mm.get_page_table().get_top_addr() as *mut u8;

    let later_used_start_page = args.heap.used_end_page;
    let later_used_end_page = heap.get().mm.get_page_table().get_used_end_page();

    // Donate page from one that is used currently
    let page_table = heap.get().mm.get_page_table();
    for page in later_used_start_page..later_used_end_page {
        let page = page_table.get_page(page).lock();
        let Some(page) = page.as_ref() else {
            continue;
        };

        args.second_mm.get_page_table_mut().donate_page(page);
    }

    let registry_frozen = args.relocation_registry.freeze();
    let compacted_end_page = page_table.get_used_end_page();

    section_cookie.section("Fix root", |_| {
        heap.get().contexts.get_mut().iter().for_each(|x| {
            let mut root_set = x.1.lock();
            let root_set = &mut root_set.root_set;

            root_set.map_pointers(&mut |x| {
                let mapped = registry_frozen
                    .map_src_to_dest(x.to_ptr().addr() - args.heap.start.addr())
                    .expect("Cannot find relocation record");
                // SAFETY: This points to correct address after relocated
                // and relocation registry contains only offsets into heap
                unsafe { ObjectPtr::new(args.heap.to_space.wrapping_byte_add(mapped)) }
            });
        });
    });

    section_cookie.section("Wipe dead types", |_| {
        heap.get().type_manager.type_manager.wipe_deads()
    });

    let copier = args.common.gc.copier.take().unwrap();

    let heap_info = HeapInfoLater {
        start: args.heap.start,
        size: args.heap.size,
        used_end: args.heap.used_end,
        used_end_page: args.heap.used_end_page,
        later_used_end,
        later_used_end_page,
        compacted_end: args.compacted_end,
        compacted_end_page,
        to_space: args.heap.to_space,
    };

    let from_space = mem::replace(&mut heap.get().mm, args.second_mm);

    // SAFETY: We're in STW that mean the heap is unused and available for exclusive access by copier
    let active_copier = section_cookie.section("Prepare copier", |_| unsafe {
        copier.start(
            heap_info.clone(),
            registry_frozen,
            from_space,
            heap.get().mm.get_page_table_cloned(),
            &heap.get().type_manager,
        )
    });

    Step4Args {
        common: args.common,
        copier: active_copier,
        heap: heap_info,
    }
}

struct Step4Args<'a> {
    common: CommonArgs<'a>,
    copier: CopierActive,
    #[expect(unused)]
    heap: HeapInfoLater,
}

/// Step 4: (Concurrent) Relocate
fn step4<'a>(section_cookie: &mut SectionCookie, mut args: Step4Args<'a>) -> Step5Args<'a> {
    let heap = args.common.shared.get_shared();

    let (registry, copier, mut second_mm) = args.copier.finish(&heap.get().type_manager);
    args.common.gc.cached_registry = Some(registry.unfreeze());
    args.common.gc.copier = Some(copier);

    section_cookie.section("Clearing from-space", |_| second_mm.clear());

    // nothing, because actual relocation is not implemented yet
    Step5Args {
        common: args.common,
        second_mm,
    }
}

struct Step5Args<'a> {
    common: CommonArgs<'a>,
    second_mm: MM,
}

/// Step 5: (STW) Finalize cycle
fn step5(_section_cookie: &mut SectionCookie, args: Step5Args<'_>) -> PersistentState {
    let mut heap = args.common.shared.get_exclusive();
    assert!(
        heap.get().second_mm.is_none(),
        "second mm already exists for no reason"
    );
    heap.get().second_mm = Some(args.second_mm);

    args.common.gc
}
