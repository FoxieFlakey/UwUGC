// Follows similar formatting like one used in Minecraft 1.17
// debug command see https://docs.minecraftforge.net/en/1.17.x/gettingstarted/debugprofiler/
//
// NOTE: The ABI or format of its is unstable, it is mainly used
// for debugging and not runtime calculating of something!

use std::{
    io::{LineWriter, Write},
    rc::Rc,
    time::{Duration, Instant},
};

use indexmap::IndexMap;

pub struct Profiler {
    root_section: Section,
}

struct Section {
    name: Rc<String>,
    this: Duration,
    total: Duration,
    subsections: Option<IndexMap<Rc<String>, Section>>,
}

pub struct SectionCookie<'a> {
    section: &'a mut Section,
    start_of_self: Instant,
}

impl<'a> SectionCookie<'a> {
    pub fn section<R, F: FnOnce(&mut SectionCookie) -> R>(&mut self, name: &str, func: F) -> R {
        let name = Rc::new(String::from(name));
        let start = Instant::now();
        self.section.this += start - self.start_of_self;

        let mut cookie = SectionCookie {
            section: self
                .section
                .subsections
                .as_mut()
                .unwrap()
                .entry(name.clone())
                .or_insert_with(|| Section::new(name)),
            start_of_self: start,
        };
        let ret = func(&mut cookie);
        let end = Instant::now();

        cookie.section.this += end - cookie.start_of_self;
        cookie.section.total += end - start;
        self.start_of_self = end;
        ret
    }
}

impl Profiler {
    pub fn new() -> Self {
        Self {
            root_section: Section::new(Rc::new("root".into())),
        }
    }

    pub fn start<R, F: FnOnce(&mut SectionCookie) -> R>(&mut self, func: F) -> R {
        let start = Instant::now();
        let mut cookie = SectionCookie {
            start_of_self: start,
            section: &mut self.root_section,
        };
        let ret = func(&mut cookie);
        let end = Instant::now();

        cookie.section.this += end - cookie.start_of_self;
        cookie.section.total = end - start;
        ret
    }
}

const ADVANCE_MODE: bool = false;

impl Profiler {
    pub fn report<Writer: Write>(&mut self, mut output: &mut LineWriter<Writer>) {
        // Sample
        // [00] root             - Parent: 100.00% Self:     1.00 ms ( 50.00%) Total:     2.00 ms
        let root_total_time = self.root_section.total.as_millis_f32();
        self.iter_section_recursive_mut(|section, parent, depth| {
            if ADVANCE_MODE {
                let padding = usize::try_from(16u32.checked_sub(depth * 2).unwrap_or(0)).unwrap();
                let mut filling = String::new();
                for _ in 0..depth {
                    filling.push_str("| ");
                }
                writeln!(&mut output,
                "[{:02}] {filling}{:<padding$} - Self time % from overall: {:#6.2}% Time % from overall: {:#6.2}% Parent %: {:#6.2}% Self: {:#8.2} ms ({:6.2}% from total) Total: {:#8.2} ms",
                depth,
                section.name,
                section.this.as_millis_f32() / root_total_time * 100.0,
                section.total.as_millis_f32() / root_total_time * 100.0,
                parent.map(|x| section.total.as_millis_f32() / x.total.as_millis_f32()).unwrap_or(1.0) * 100.0,
                section.this.as_millis_f32(),
                section.this.as_millis_f32() / section.total.as_millis_f32() * 100.0,
                section.total.as_millis_f32()
                ).unwrap();
            } else {
                let padding = usize::try_from(16u32.checked_sub(depth * 2).unwrap_or(0)).unwrap();
                let mut filling = String::new();
                for _ in 0..depth {
                filling.push_str("| ");
                }
                writeln!(&mut output,
                "[{:02}] {filling}{:<padding$} - {:6.2}%/{:6.2}% {:#8.2} ms",
                depth,
                section.name,
                parent.map(|x| section.total.as_millis_f32() / x.total.as_millis_f32()).unwrap_or(1.0) * 100.0,
                section.total.as_millis_f32() / root_total_time * 100.0,
                section.total.as_millis_f32()
                ).unwrap();
            }
        });
    }

    fn iter_section_recursive_mut<F: FnMut(&mut Section, Option<&Section>, u32)>(
        &mut self,
        mut func: F,
    ) {
        let mut depth = 0;
        let mut stack = Vec::new();
        func(&mut self.root_section, None, depth);
        stack.push((
            self.root_section.clone_without_subsections(),
            self.root_section.subsections.as_mut().unwrap().values_mut(),
        ));
        depth += 1;

        while let Some(tail) = stack.last_mut() {
            let Some(item) = tail.1.next() else {
                depth -= 1;
                stack.pop();
                continue;
            };

            func(item, Some(&tail.0), depth);
            stack.push((
                item.clone_without_subsections(),
                item.subsections.as_mut().unwrap().values_mut(),
            ));
            depth += 1;
        }
    }
}

impl Section {
    fn new(name: Rc<String>) -> Self {
        Self {
            this: Duration::ZERO,
            total: Duration::ZERO,
            subsections: Some(IndexMap::new()),
            name,
        }
    }

    fn clone_without_subsections(&self) -> Self {
        Self {
            subsections: None,
            name: self.name.clone(),
            this: self.this,
            total: self.total,
        }
    }
}
