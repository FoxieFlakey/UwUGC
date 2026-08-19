use std::{
    borrow::Cow,
    collections::HashMap,
    ptr::{self, NonNull},
    sync::atomic::{AtomicPtr, AtomicU64, Ordering},
};

use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard};
use uwugc::{ObjectPtr, TypeManager};

use crate::Descriptor;

mod pointer_iterator;
pub use pointer_iterator::PointerIterator;

// So GC gave 64-bit payload and then this module reserves 2 bits at bottom
// for kind
// 0b00 => static type, a.k.a it points to &'static Descriptor, can immediately
//  be dereferenced
// 0b01 => ref array, its a array of GCBoxs
// 0b10 => dynamic added type

static PAYLOAD_SHIFT: u32 = 2;
static TYPE_TYPE_MASK: u64 = 0b11;

pub struct Types {
    latest_id: AtomicU64,
    dynamic_types: RwLock<HashMap<TypeId, DescriptorEntry>>,
}

struct DescriptorEntry {
    marked_dead: bool,
    descriptor: Cow<'static, Descriptor>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct TypeId(pub u64);

impl From<&'static Descriptor> for TypeId {
    fn from(value: &'static Descriptor) -> Self {
        let ptr = u64::try_from(ptr::from_ref(value).addr()).unwrap();
        assert_eq!(
            ptr & TYPE_TYPE_MASK,
            0,
            "Pointer to descriptor aren't properly aligned bottom {PAYLOAD_SHIFT} bits is used when it shouldnt"
        );
        TypeId(ptr)
    }
}

impl Types {
    pub(crate) fn new() -> Self {
        Self {
            latest_id: AtomicU64::new(0),
            dynamic_types: RwLock::new(HashMap::new()),
        }
    }

    // NOTE: newly registered descriptor may suddenly disappear. If you
    // registered while not having any GC context as concurrent GC may removes
    // the descriptor.
    pub fn register(&self, descriptor: Cow<'static, Descriptor>) -> TypeId {
        let latest_id = self
            .latest_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_add(1))
            .ok()
            .map(TypeId)
            .expect("Ran out of IDs");
        let mut map = self.dynamic_types.write();
        map.try_insert(
            latest_id,
            DescriptorEntry {
                marked_dead: true,
                descriptor,
            },
        )
        .ok()
        .expect("There must not be existing entry, we just got unique ID by atomic");

        latest_id
    }

    pub fn get_type<'a>(&'a self, type_id: TypeId) -> Option<TypeInfo<'a>> {
        let type_id = type_id.0;
        let type_type = type_id & TYPE_TYPE_MASK;
        match type_type {
            0b00 => {
                // Static type, the type_id contains pointer to &'static Descriptor
                let ptr = ptr::with_exposed_provenance_mut::<Descriptor>(
                    usize::try_from(type_id & (!TYPE_TYPE_MASK)).unwrap(),
                );

                // SAFETY: we control alll type_id that exists in GC heap
                let desc = unsafe { ptr.as_ref() }.unwrap();
                Some(TypeInfoImpl::StaticallyKnown(desc))
            }

            0b01 => {
                // This is ref array we can calculate the size of it directly
                Some(TypeInfoImpl::RefArray(
                    usize::try_from(type_id >> PAYLOAD_SHIFT).unwrap(),
                ))
            }

            0b10 => {
                let guard = self.dynamic_types.read();

                let Ok(descriptor) = RwLockReadGuard::try_map(guard, |map| {
                    map.get(&TypeId(type_id)).map(|x| &*x.descriptor)
                }) else {
                    // The type id is unknown
                    return None;
                };

                Some(TypeInfoImpl::DynamicallyKnown(descriptor))
            }

            0b11 => unimplemented!("No idea what bit pattern 0b11 should be"),

            _ => unreachable!(),
        }
        .map(TypeInfo)
    }
}

enum TypeInfoImpl<'a> {
    StaticallyKnown(&'static Descriptor),
    RefArray(usize),
    DynamicallyKnown(MappedRwLockReadGuard<'a, Descriptor>),
}

pub struct TypeInfo<'a>(TypeInfoImpl<'a>);

impl<'a> TypeInfo<'a> {
    // This return offset to each GC pointer
    pub fn iter_pointers(&'a self) -> PointerIterator<'a> {
        PointerIterator::from_type_info(self)
    }

    pub fn get_size(&self) -> usize {
        match &self.0 {
            TypeInfoImpl::RefArray(len) => len * size_of::<*mut u8>(),
            TypeInfoImpl::DynamicallyKnown(desc) => desc.size,
            TypeInfoImpl::StaticallyKnown(desc) => desc.size,
        }
    }
}

unsafe impl TypeManager for Types {
    fn assume_all_dead(&mut self) {
        self.dynamic_types
            .get_mut()
            .values_mut()
            .for_each(|x| x.marked_dead = true);
    }

    fn get_size(&self, type_id: u64) -> Option<usize> {
        self.get_type(TypeId(type_id)).map(|x| x.get_size())
    }

    fn iterate_gc_pointers(
        &self,
        type_id: u64,
        object: uwugc::ObjectPtr,
        visitor: &mut dyn FnMut(uwugc::ObjectPtr),
    ) -> bool {
        let Some(fields_iter) = self.get_type(TypeId(type_id)) else {
            return false;
        };

        for field in fields_iter.iter_pointers() {
            // SAFETY: We got field offset frm trusted sources like RefArray its essentially
            // every entry in array and for descriptor, descriptor maker already make sure its safe
            let field = unsafe { object.data().byte_add(field) }.cast::<AtomicPtr<u8>>();
            // SAFETY: Registrator make sure offset is correct and GC make sure that
            // the 'object' cover valid range and contains the field
            let field = unsafe { field.as_ref() }.load(Ordering::Relaxed);
            if let Some(val) = NonNull::new(field) {
                // SAFETY: The content of each fields are controlled by us
                // via GCBox and caller make sure it only ever contains valid
                // GC pointer
                visitor(unsafe { ObjectPtr::from_nonnull(val) });
            }
        }

        true
    }

    fn set_alive(&self, type_id: u64) -> bool {
        self.dynamic_types
            .write()
            .get_mut(&TypeId(type_id))
            .map(|x| x.marked_dead = false)
            .is_some()
    }

    fn update_gc_pointers(
        &self,
        type_id: u64,
        object: uwugc::ObjectPtr,
        updater: &mut dyn FnMut(uwugc::ObjectPtr) -> uwugc::ObjectPtr,
    ) -> bool {
        let Some(type_info) = self.get_type(TypeId(type_id)) else {
            return false;
        };

        for field in type_info.iter_pointers() {
            // SAFETY: registering descriptor requires caller to make sure offsets
            // are valid for .byte_add and within the object size. So this is safe
            let mut field = unsafe { object.data().byte_add(field) }.cast::<*mut u8>();
            // SAFETY: Registrator make sure offset is correct and GC make sure that
            // the 'object' cover valid range and contains the field
            let field = unsafe { field.as_mut() };
            if let Some(current_val) = NonNull::new(*field) {
                // SAFETY: The content of each fields are controlled by us
                // via GCBox and caller make sure it only ever contains valid
                // GC pointer
                *field = updater(unsafe { ObjectPtr::from_nonnull(current_val) })
                    .into_raw()
                    .as_ptr();
            }
        }

        true
    }

    fn wipe_deads(&mut self) {
        self.dynamic_types.get_mut().retain(|_, v| !v.marked_dead);
    }
}
