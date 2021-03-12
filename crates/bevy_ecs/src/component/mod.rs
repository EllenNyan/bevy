mod type_info;

pub use type_info::*;

use std::collections::HashMap;

use crate::storage::SparseSetIndex;
use bitflags::bitflags;
use std::{
    alloc::Layout,
    any::{Any, TypeId},
    collections::hash_map::Entry,
};
use thiserror::Error;

pub trait Component: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Component for T {}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StorageType {
    Table,
    SparseSet,
}

impl Default for StorageType {
    fn default() -> Self {
        StorageType::Table
    }
}

#[derive(Debug)]
pub struct ComponentDescriptor {
    name: String,
    storage_type: StorageType,
    // SAFETY: This must remain private. It must only be set to "true" if this component is actually Send + Sync
    is_send_and_sync: bool,
    type_id: Option<TypeId>,
    layout: Layout,
    drop: unsafe fn(*mut u8),
}

impl ComponentDescriptor {
    pub fn new<T: Component>(storage_type: StorageType) -> Self {
        Self {
            name: std::any::type_name::<T>().to_string(),
            storage_type,
            is_send_and_sync: true,
            type_id: Some(TypeId::of::<T>()),
            layout: Layout::new::<T>(),
            drop: TypeInfo::drop_ptr::<T>,
        }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn type_id(&self) -> Option<TypeId> {
        self.type_id
    }

    #[inline]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    #[inline]
    pub fn drop(&self) -> unsafe fn(*mut u8) {
        self.drop
    }

    #[inline]
    pub fn storage_type(&self) -> StorageType {
        self.storage_type
    }

    #[inline]
    pub fn is_send_and_sync(&self) -> bool {
        self.is_send_and_sync
    }
}

impl From<TypeInfo> for ComponentDescriptor {
    fn from(type_info: TypeInfo) -> Self {
        Self {
            name: type_info.type_name().to_string(),
            storage_type: StorageType::default(),
            is_send_and_sync: type_info.is_send_and_sync(),
            type_id: Some(type_info.type_id()),
            drop: type_info.drop(),
            layout: type_info.layout(),
        }
    }
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct ComponentId(usize);

impl ComponentId {
    #[inline]
    pub const fn new(index: usize) -> ComponentId {
        ComponentId(index)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0
    }
}

impl SparseSetIndex for ComponentId {
    #[inline]
    fn sparse_set_index(&self) -> usize {
        self.index()
    }

    fn get_sparse_set_index(value: usize) -> Self {
        Self(value)
    }
}

pub mod default_relationship_kinds {
    pub struct HasComponent;
    pub struct HasResource;
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub enum DummyIdOrEntity {
    Entity(crate::entity::Entity),
    DummyId(DummyId),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct Relationship {
    kind: RelationshipKindId,
    target: DummyIdOrEntity,
}

impl Relationship {
    pub fn new(kind: RelationshipKindId, target: DummyIdOrEntity) -> Self {
        Self { kind, target }
    }
}

#[derive(Debug)]
pub struct ComponentInfo {
    id: ComponentId,
    relationship: Relationship,
    data: ComponentDescriptor,
}

impl ComponentInfo {
    pub fn id(&self) -> ComponentId {
        self.id
    }

    pub fn get_component_descriptor(&self) -> &ComponentDescriptor {
        &self.data
    }
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct RelationshipKindId(usize);
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct RelationshipKindInfo {
    id: RelationshipKindId,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct DummyIdInfo {
    static_type: Option<TypeId>,
    id: DummyId,
}
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct DummyId(usize);

#[derive(Debug)]
pub struct Components {
    components: Vec<ComponentInfo>,
    component_indices: HashMap<Relationship, ComponentId, fxhash::FxBuildHasher>,

    kinds: Vec<RelationshipKindInfo>,

    dummy_infos: Vec<DummyIdInfo>,
    // FIXME(Relationships) we use TypeId here because bevy needs to do this conversion *somewhere*
    // scripting stuff can directly use ComponentId/RelationshipKindId somehow
    typeid_to_component_id: HashMap<TypeId, DummyId, fxhash::FxBuildHasher>,
    kind_indices: HashMap<TypeId, RelationshipKindId, fxhash::FxBuildHasher>,
}

impl Default for Components {
    fn default() -> Self {
        let mut this = Self {
            components: Default::default(),
            component_indices: Default::default(),

            kinds: Default::default(),

            dummy_infos: Default::default(),
            typeid_to_component_id: Default::default(),
            kind_indices: Default::default(),
        };

        this.new_relationship_kind(Some(
            TypeId::of::<default_relationship_kinds::HasComponent>(),
        ));
        this.new_relationship_kind(Some(TypeId::of::<default_relationship_kinds::HasResource>()));

        this
    }
}

#[derive(Debug, Error)]
pub enum ComponentsError {
    #[error("A component of type {0:?} already exists")]
    ComponentAlreadyExists(Relationship),
}

impl Components {
    pub fn relkind_of_has_component(&self) -> RelationshipKindId {
        self.kind_indices[&TypeId::of::<default_relationship_kinds::HasComponent>()]
    }

    pub fn relkind_of_has_resource(&self) -> RelationshipKindId {
        self.kind_indices[&TypeId::of::<default_relationship_kinds::HasResource>()]
    }

    /// TypeId is used for bevy to map from relationship kind structs -> RelationshipKindId  
    /// scripting/untyped use of this should pass in None
    pub fn new_relationship_kind(&mut self, type_id: Option<TypeId>) -> RelationshipKindId {
        let id = RelationshipKindId(self.kinds.len());

        if let Some(type_id) = type_id {
            let previously_inserted = self
                .kind_indices
                .insert(type_id, RelationshipKindId(self.kinds.len()));
            assert!(previously_inserted.is_none());
        }

        self.kinds.push(RelationshipKindInfo { id });
        id
    }

    /// TypeId is used by bevy to map from component type -> component id  
    /// scripting/untyped use of this should pass in None  
    pub(crate) fn new_component_id(&mut self, type_id: Option<TypeId>) -> DummyId {
        let component_id = DummyId(self.dummy_infos.len());
        self.dummy_infos.push(DummyIdInfo {
            static_type: type_id,
            id: component_id,
        });

        if let Some(type_id) = type_id {
            let previously_inserted = self.typeid_to_component_id.insert(type_id, component_id);
            assert!(previously_inserted.is_none());
        }
        component_id
    }

    pub(crate) fn type_id_to_component_id(&self, type_id: TypeId) -> Option<DummyId> {
        self.typeid_to_component_id.get(&type_id).copied()
    }

    pub(crate) fn register_relationship(
        &mut self,
        relationship: Relationship,
        comp_descriptor: ComponentDescriptor,
    ) -> Result<&ComponentInfo, ComponentsError> {
        let new_id = ComponentId(self.components.len());

        if let Entry::Occupied(_) = self.component_indices.entry(relationship) {
            return Err(ComponentsError::ComponentAlreadyExists(relationship));
        }

        if let Some(type_id) = comp_descriptor.type_id {
            if let DummyIdOrEntity::DummyId(id) = relationship.target {
                if let Some(stored_type_id) = self.dummy_infos[id.0].static_type {
                    assert!(stored_type_id == type_id);
                }

                let component_id = self.typeid_to_component_id[&type_id];
                assert!(DummyIdOrEntity::DummyId(component_id) == relationship.target);
            }
        }

        self.component_indices.insert(relationship, new_id);
        self.components.push(ComponentInfo {
            id: new_id,
            relationship,
            data: comp_descriptor,
        });

        // Safety: Just inserted ^^^
        unsafe { Ok(self.get_relationship_info_unchecked(new_id)) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.components.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.components.len() == 0
    }

    #[inline]
    pub fn get_resource_id(&self, type_id: TypeId) -> Option<ComponentId> {
        self.get_relationship_id(Relationship {
            kind: self.relkind_of_has_resource(),
            target: DummyIdOrEntity::DummyId(self.type_id_to_component_id(type_id)?),
        })
    }
    #[inline]
    pub fn get_resource_info_or_insert<T: Component>(&mut self) -> &ComponentInfo {
        self.get_resource_info_or_insert_with(TypeId::of::<T>(), TypeInfo::of::<T>)
    }
    #[inline]
    pub fn get_non_send_resource_info_or_insert<T: Any>(&mut self) -> &ComponentInfo {
        self.get_resource_info_or_insert_with(
            TypeId::of::<T>(),
            TypeInfo::of_non_send_and_sync::<T>,
        )
    }
    #[inline]
    fn get_resource_info_or_insert_with(
        &mut self,
        type_id: TypeId,
        data_layout: impl FnOnce() -> TypeInfo,
    ) -> &ComponentInfo {
        let component_id = match self.type_id_to_component_id(type_id) {
            Some(id) => id,
            None => self.new_component_id(Some(type_id)),
        };

        self.get_relationship_info_or_insert_with(
            Relationship {
                kind: self.relkind_of_has_resource(),
                target: DummyIdOrEntity::DummyId(component_id),
            },
            data_layout,
        )
    }

    #[inline]
    pub fn get_component_id(&self, type_id: TypeId) -> Option<ComponentId> {
        self.get_relationship_id(Relationship {
            kind: self.relkind_of_has_component(),
            target: DummyIdOrEntity::DummyId(self.type_id_to_component_id(type_id)?),
        })
    }
    #[inline]
    pub fn get_component_info_or_insert<T: Component>(&mut self) -> &ComponentInfo {
        self.get_component_info_or_insert_with(TypeId::of::<T>(), TypeInfo::of::<T>)
    }
    #[inline]
    pub(crate) fn get_component_info_or_insert_with(
        &mut self,
        type_id: TypeId,
        data_layout: impl FnOnce() -> TypeInfo,
    ) -> &ComponentInfo {
        let component_id = match self.type_id_to_component_id(type_id) {
            Some(id) => id,
            None => self.new_component_id(Some(type_id)),
        };

        self.get_relationship_info_or_insert_with(
            Relationship {
                kind: self.relkind_of_has_component(),
                target: DummyIdOrEntity::DummyId(component_id),
            },
            data_layout,
        )
    }

    #[inline]
    pub fn get_relationship_id(&self, relationship: Relationship) -> Option<ComponentId> {
        self.component_indices.get(&relationship).copied()
    }
    #[inline]
    pub fn get_relationship_info(&self, id: ComponentId) -> Option<&ComponentInfo> {
        self.components.get(id.0)
    }
    /// # Safety
    /// `id` must be a valid [RelationshipId]
    #[inline]
    pub unsafe fn get_relationship_info_unchecked(&self, id: ComponentId) -> &ComponentInfo {
        debug_assert!(id.index() < self.components.len());
        self.components.get_unchecked(id.0)
    }
    #[inline]
    pub fn get_relationship_info_or_insert_with(
        &mut self,
        relationship: Relationship,
        data_layout: impl FnOnce() -> TypeInfo,
    ) -> &ComponentInfo {
        let Components {
            component_indices: relationship_indices,
            components: relationships,
            ..
        } = self;

        let id = *relationship_indices.entry(relationship).or_insert_with(|| {
            let rel_id = ComponentId(relationships.len());

            relationships.push(ComponentInfo {
                id: rel_id,
                relationship,
                data: data_layout().into(),
            });

            rel_id
        });

        // Safety: just inserted
        unsafe { self.get_relationship_info_unchecked(id) }
    }
}

bitflags! {
    pub struct ComponentFlags: u8 {
        const ADDED = 1;
        const MUTATED = 2;
    }
}
