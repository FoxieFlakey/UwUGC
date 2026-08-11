use arbitrary_int::traits::Integer;

use crate::object::{MetadataCompressed, ObjectKind, ObjectPtr};

// A trait user of GC implements to teach GC
// where offsets to each pointer GC need to
// know is.
//
// type_id here refers to unique identifier for
// each type manager knows. It responsible for
// knowing how big an object is, the positions
// of each pointer GC need to be aware. etc
//
// # Safety
// Result must be the same as long as atleast
// one object referring same type_id exists. That
// means type_id can be reused ONLY if no other
// object refers to it. Failure to do that will lead
// to GC fuck up things -w-
//
// Do not assume ObjectPtr's address values at all.
// Only thing its safe is start of an object to end
// object.
pub unsafe trait TypeManager: Send + Sync {
    // GC tells that all descriptors are "dead"
    // but do not remove all desciptors yet
    //
    // This is mainly due my GC, does mark-compact
    // so there no sweep over dead objects at all
    // so it cant just say "dead_descriptor(...)"
    //
    // What it can do is assume all objects are dead
    // then set_alive on each descriptors at end of
    // cycle GC going to call "wipe_deads" (it will
    // be done in STW phase. so keep it short)
    //
    // This is called in STW phase, keep it short
    // you can see the pattern of &mut mean STW
    // else just & means concurrent
    fn assume_all_dead(&mut self);

    // This object is alive. Specifically only
    // MetadataEnum::PlainOldData will result
    // calling to this.
    //
    // Returns true if type_id is exists and marked
    // else false if not
    #[must_use = "The type_id might not exists"]
    #[expect(unused)]
    fn set_alive(&self, type_id: u64) -> bool;

    // Actually start wiping the dead type_ids
    // that isnt marked by set_alive
    fn wipe_deads(&mut self);

    // Iterate GC pointers present in an object. NOTE
    // mutator may modifies the pointer concurrently.
    //
    // Weird request but MUST only access fields that
    // is safe to be accessed while there &mut. Only
    // one field that work is Atomic* fields because
    // LLVM won't turn atomic into non atomics even at
    // face of &mut (or noalias to the LLVM). With
    // one request .get_mut is disallowed anywhere as
    // long as GC can see. Must only use .load or .store
    //
    // This also has to be READ ONLY to the object. The
    // underlying memory on the object itself MAY BE
    // mapped without PROT_WRITE.
    //
    // Returns true if type_id is exists and marked
    // else false if not
    fn iterate_gc_pointers(
        &self,
        type_id: u64,
        object: ObjectPtr,
        visitor: &mut dyn FnMut(ObjectPtr),
    ) -> bool;

    // Update GC pointers present in an object. NOTE for
    // safetyness: at here the 'object'. It is guarantee
    // that object is exclusively owned here so regular
    // store/load on GC field is safe
    //
    // Returns true if type_id is exists and marked
    // else false if not
    fn update_gc_pointers(
        &self,
        type_id: u64,
        object: ObjectPtr,
        updater: &mut dyn FnMut(ObjectPtr) -> ObjectPtr,
    ) -> bool;

    // Get size of type_id. Excluding metadata
    // Returns None if type_id unknown else Some(length)
    fn get_size(&self, type_id: u64) -> Option<usize>;
}

pub struct NoopTypeManager;

unsafe impl TypeManager for NoopTypeManager {
    fn assume_all_dead(&mut self) {}
    fn get_size(&self, _: u64) -> Option<usize> {
        None
    }

    fn set_alive(&self, _: u64) -> bool {
        false
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

    fn wipe_deads(&mut self) {}
}

pub struct TypeManagerConcrete {
    pub type_manager: Box<dyn TypeManager>,
}

impl TypeManagerConcrete {
    pub fn new<M: TypeManager + 'static>(manager: M) -> Self {
        Self {
            type_manager: Box::new(manager),
        }
    }

    #[expect(unused)]
    pub fn iterate_gc_pointers(&self, object: ObjectPtr, visitor: &mut dyn FnMut(ObjectPtr)) {
        match object.metadata().payload {
            ObjectKind::PlainOldData(_) => (),
            ObjectKind::NotPlainOldData(payload) => {
                assert!(
                    self.type_manager
                        .iterate_gc_pointers(payload.as_u64(), object, visitor)
                );
            }
        }
    }

    // Get size of object including header
    pub fn get_size(&self, object: &ObjectPtr) -> usize {
        (match object.metadata().payload {
            ObjectKind::PlainOldData(x) => x.as_usize(),
            ObjectKind::NotPlainOldData(type_id) => {
                self.type_manager.get_size(type_id.as_u64()).unwrap()
            }
        }) + size_of::<MetadataCompressed>()
    }

    // # Safety
    // caller must make sure object is exclusive owned and its
    // safe from GC accessing part of it.
    #[expect(unused)]
    pub unsafe fn update_gc_pointers(
        &self,
        object: ObjectPtr,
        updater: &mut dyn FnMut(ObjectPtr) -> ObjectPtr,
    ) {
        match object.metadata().payload {
            ObjectKind::PlainOldData(_) => (),
            ObjectKind::NotPlainOldData(payload) => {
                assert!(
                    self.type_manager
                        .update_gc_pointers(payload.as_u64(), object, updater)
                );
            }
        }
    }
}
