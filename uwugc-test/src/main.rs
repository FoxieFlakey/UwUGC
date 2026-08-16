use uwugc::{ObjectPtr, TypeManager, UwUGC};

mod gclatencytest;

fn main() {
    let mut state = UwUGC::new(
        512 * 1024 * 1024,
        Some(0x60ef_0000_0000),
        Some(0x60ff_0000_0000),
        NoopTypeManager,
    )
    .unwrap();

    gclatencytest::run(&mut state);
}

pub struct NoopTypeManager;

unsafe impl TypeManager for NoopTypeManager {
    fn assume_all_dead(&mut self) {}
    fn wipe_deads(&mut self) {}
    fn set_alive(&self, _: u64) -> bool {
        false
    }
    fn get_size(&self, _: u64) -> Option<usize> {
        None
    }
    fn iterate_gc_pointers(&self, _: u64, _: ObjectPtr, _: &mut dyn FnMut(ObjectPtr)) -> bool {
        false
    }
    fn update_gc_pointers(
        &self,
        _: u64,
        _: ObjectPtr,
        _: &mut dyn FnMut(ObjectPtr) -> ObjectPtr,
    ) -> bool {
        false
    }
}
