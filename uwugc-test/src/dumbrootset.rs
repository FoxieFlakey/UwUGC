use uwugc::{ObjectPtr, RootSet};

#[derive(Clone)]
pub struct DumbRootSet {
    set: Vec<Option<ObjectPtr>>,
}

impl DumbRootSet {
    pub fn new(size: usize) -> Self {
        let mut set = Vec::new();
        set.resize(size, None);
        Self { set }
    }

    // shared reference ensures no mutable reference to the memory
    pub fn as_slice<'a>(&'a self) -> &'a [Option<ObjectPtr>] {
        &self.set
    }

    // &mut ensure nothing accesses the memory
    pub fn as_slice_mut<'a>(&'a mut self) -> &'a mut [Option<ObjectPtr>] {
        &mut self.set
    }
}

unsafe impl RootSet for DumbRootSet {
    fn clone_boxed(&self) -> Box<dyn RootSet> {
        Box::new(self.clone())
    }

    fn iter_pointers(&self, visitor: &mut dyn FnMut(&ObjectPtr)) {
        self.as_slice().iter().flatten().for_each(visitor);
    }

    fn map_pointers(&mut self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr) {
        self.as_slice_mut().iter_mut().flatten().for_each(|x| {
            *x = visitor(*x);
        });
    }
}
