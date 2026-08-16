use std::any::Any;

use crate::object::ObjectPtr;

// # Safety
// caller has to make the ObjectPtr given to GC is valid one from
// GC else GC willl be unsafe
pub unsafe trait RootSet: Send + Any {
    fn map_pointers(&mut self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr);
    fn iter_pointers(&self, visitor: &mut dyn FnMut(&ObjectPtr));
    fn clone_boxed(&self) -> Box<dyn RootSet>;
}

