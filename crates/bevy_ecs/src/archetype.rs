use bevy_utils::HashMap;
use bevy_utils::StableHashMap;

use crate::{
    bundle::BundleId,
    component::{EntityAtomKindId, StorageType},
    entity::{Entity, EntityLocation},
    storage::{Column, SparseArray, SparseSet, SparseSetIndex, TableId},
};
use std::{
    borrow::Cow,
    hash::Hash,
    ops::{Index, IndexMut},
};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct ArchetypeId(usize);

impl ArchetypeId {
    #[inline]
    pub const fn new(index: usize) -> Self {
        ArchetypeId(index)
    }

    #[inline]
    pub const fn empty() -> ArchetypeId {
        ArchetypeId(0)
    }

    #[inline]
    pub const fn resource() -> ArchetypeId {
        ArchetypeId(1)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0
    }
}

pub enum ComponentStatus {
    Added,
    Mutated,
}

pub struct AddBundle {
    pub archetype_id: ArchetypeId,
    pub bundle_status: Vec<ComponentStatus>,
}

#[derive(Default)]
pub struct Edges {
    pub add_bundle: SparseArray<BundleId, AddBundle>,
    pub remove_bundle: SparseArray<BundleId, Option<ArchetypeId>>,
    pub remove_bundle_intersection: SparseArray<BundleId, Option<ArchetypeId>>,
}

impl Edges {
    #[inline]
    pub fn get_add_bundle(&self, bundle_id: BundleId) -> Option<&AddBundle> {
        self.add_bundle.get(bundle_id)
    }

    #[inline]
    pub fn set_add_bundle(
        &mut self,
        bundle_id: BundleId,
        archetype_id: ArchetypeId,
        bundle_status: Vec<ComponentStatus>,
    ) {
        self.add_bundle.insert(
            bundle_id,
            AddBundle {
                archetype_id,
                bundle_status,
            },
        );
    }

    #[inline]
    pub fn get_remove_bundle(&self, bundle_id: BundleId) -> Option<Option<ArchetypeId>> {
        self.remove_bundle.get(bundle_id).cloned()
    }

    #[inline]
    pub fn set_remove_bundle(&mut self, bundle_id: BundleId, archetype_id: Option<ArchetypeId>) {
        self.remove_bundle.insert(bundle_id, archetype_id);
    }

    #[inline]
    pub fn get_remove_bundle_intersection(
        &self,
        bundle_id: BundleId,
    ) -> Option<Option<ArchetypeId>> {
        self.remove_bundle_intersection.get(bundle_id).cloned()
    }

    #[inline]
    pub fn set_remove_bundle_intersection(
        &mut self,
        bundle_id: BundleId,
        archetype_id: Option<ArchetypeId>,
    ) {
        self.remove_bundle_intersection
            .insert(bundle_id, archetype_id);
    }
}

struct TableInfo {
    id: TableId,
    entity_rows: Vec<usize>,
}

pub(crate) struct ArchetypeSwapRemoveResult {
    pub swapped_entity: Option<Entity>,
    pub table_row: usize,
}

pub(crate) struct ArchetypeComponentInfo {
    pub(crate) storage_type: StorageType,
    pub(crate) archetype_component_id: ArchetypeComponentId,
}

pub struct Archetype {
    id: ArchetypeId,
    entities: Vec<Entity>,
    edges: Edges,
    table_info: TableInfo,
    table_components: Cow<'static, [(EntityAtomKindId, Option<Entity>)]>,
    sparse_set_components: Cow<'static, [(EntityAtomKindId, Option<Entity>)]>,
    pub(crate) unique_components: SparseSet<EntityAtomKindId, Column>,
    pub(crate) components: SparseSet<EntityAtomKindId, ArchetypeComponentInfo>,
    pub(crate) relations: SparseSet<EntityAtomKindId, StableHashMap<Entity, ArchetypeComponentInfo>>,
}

impl Archetype {
    pub fn new(
        id: ArchetypeId,
        table_id: TableId,
        table_components: Cow<'static, [(EntityAtomKindId, Option<Entity>)]>,
        sparse_set_components: Cow<'static, [(EntityAtomKindId, Option<Entity>)]>,
        table_archetype_components: Vec<ArchetypeComponentId>,
        sparse_set_archetype_components: Vec<ArchetypeComponentId>,
    ) -> Self {
        // FIXME(Relationships) sort out this capacity weirdness
        let mut components =
            SparseSet::with_capacity(table_components.len() + sparse_set_components.len());
        let mut relations = SparseSet::new();
        for ((kind_id, target), archetype_component_id) in
            table_components.iter().zip(table_archetype_components)
        {
            let arch_comp_info = ArchetypeComponentInfo {
                storage_type: StorageType::Table,
                archetype_component_id,
            };

            match target {
                None => {
                    components.insert(*kind_id, arch_comp_info);
                }
                Some(target) => {
                    let set = relations.get_or_insert_with(*kind_id, StableHashMap::default);
                    set.insert(*target, arch_comp_info);
                }
            };
        }

        for ((kind_id, target), archetype_component_id) in sparse_set_components
            .iter()
            .zip(sparse_set_archetype_components)
        {
            let arch_comp_info = ArchetypeComponentInfo {
                storage_type: StorageType::SparseSet,
                archetype_component_id,
            };

            match target {
                None => {
                    components.insert(*kind_id, arch_comp_info);
                }
                Some(target) => {
                    let set = relations.get_or_insert_with(*kind_id, StableHashMap::default);
                    set.insert(*target, arch_comp_info);
                }
            };
        }

        Self {
            id,
            table_info: TableInfo {
                id: table_id,
                entity_rows: Default::default(),
            },
            components,
            relations,
            table_components,
            sparse_set_components,
            unique_components: SparseSet::new(),
            entities: Default::default(),
            edges: Default::default(),
        }
    }

    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    #[inline]
    pub fn table_id(&self) -> TableId {
        self.table_info.id
    }

    #[inline]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    #[inline]
    pub fn entity_table_rows(&self) -> &[usize] {
        &self.table_info.entity_rows
    }

    #[inline]
    pub fn table_components(&self) -> &[(EntityAtomKindId, Option<Entity>)] {
        &self.table_components
    }

    #[inline]
    pub fn sparse_set_components(&self) -> &[(EntityAtomKindId, Option<Entity>)] {
        &self.sparse_set_components
    }

    #[inline]
    pub fn unique_components(&self) -> &SparseSet<EntityAtomKindId, Column> {
        &self.unique_components
    }

    #[inline]
    pub fn unique_components_mut(&mut self) -> &mut SparseSet<EntityAtomKindId, Column> {
        &mut self.unique_components
    }

    // FIXME(Relationships) this also yields relations which feels weird but also needed
    #[inline]
    pub fn components(&self) -> impl Iterator<Item = (EntityAtomKindId, Option<Entity>)> + '_ {
        self.components
            .indices()
            .map(|kind| (kind, None))
            .chain(self.relations.indices().flat_map(move |kind_id| {
                self.relations
                    .get(kind_id)
                    .unwrap()
                    .keys()
                    .map(move |target| (kind_id, Some(*target)))
            }))
    }

    #[inline]
    pub fn edges(&self) -> &Edges {
        &self.edges
    }

    #[inline]
    pub(crate) fn edges_mut(&mut self) -> &mut Edges {
        &mut self.edges
    }

    #[inline]
    pub fn entity_table_row(&self, index: usize) -> usize {
        self.table_info.entity_rows[index]
    }

    #[inline]
    pub fn set_entity_table_row(&mut self, index: usize, table_row: usize) {
        self.table_info.entity_rows[index] = table_row;
    }

    /// # Safety
    /// valid component values must be immediately written to the relevant storages
    /// `table_row` must be valid
    pub unsafe fn allocate(&mut self, entity: Entity, table_row: usize) -> EntityLocation {
        self.entities.push(entity);
        self.table_info.entity_rows.push(table_row);

        EntityLocation {
            archetype_id: self.id,
            index: self.entities.len() - 1,
        }
    }

    pub fn reserve(&mut self, additional: usize) {
        self.entities.reserve(additional);
        self.table_info.entity_rows.reserve(additional);
    }

    /// Removes the entity at `index` by swapping it out. Returns the table row the entity is stored
    /// in.
    pub(crate) fn swap_remove(&mut self, index: usize) -> ArchetypeSwapRemoveResult {
        let is_last = index == self.entities.len() - 1;
        self.entities.swap_remove(index);
        ArchetypeSwapRemoveResult {
            swapped_entity: if is_last {
                None
            } else {
                Some(self.entities[index])
            },
            table_row: self.table_info.entity_rows.swap_remove(index),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    #[inline]
    pub fn contains(&self, relation_kind: EntityAtomKindId, relation_target: Option<Entity>) -> bool {
        match relation_target {
            None => self.components.contains(relation_kind),
            Some(target) => self
                .relations
                .get(relation_kind)
                .map(|set| set.get(&target))
                .flatten()
                .is_some(),
        }
    }

    // FIXME(Relationships) technically the target is unnecessary here as all `KindId` have the same storage type
    #[inline]
    pub fn get_storage_type(
        &self,
        relation_kind: EntityAtomKindId,
        relation_target: Option<Entity>,
    ) -> Option<StorageType> {
        match relation_target {
            None => self.components.get(relation_kind),
            Some(target) => self
                .relations
                .get(relation_kind)
                .map(|set| set.get(&target))
                .flatten(),
        }
        .map(|info| info.storage_type)
    }

    #[inline]
    pub fn get_archetype_component_id(
        &self,
        relation_kind: EntityAtomKindId,
        relation_target: Option<Entity>,
    ) -> Option<ArchetypeComponentId> {
        match relation_target {
            None => self.components.get(relation_kind),
            Some(target) => self
                .relations
                .get(relation_kind)
                .map(|set| set.get(&target))
                .flatten(),
        }
        .map(|info| info.archetype_component_id)
    }
}

/// A generational id that changes every time the set of archetypes changes
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ArchetypeGeneration(usize);

impl ArchetypeGeneration {
    #[inline]
    pub const fn initial() -> Self {
        ArchetypeGeneration(0)
    }

    #[inline]
    pub fn value(self) -> usize {
        self.0
    }
}

#[derive(Hash, PartialEq, Eq)]
pub struct ArchetypeIdentity {
    table_components: Cow<'static, [(EntityAtomKindId, Option<Entity>)]>,
    sparse_set_components: Cow<'static, [(EntityAtomKindId, Option<Entity>)]>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct ArchetypeComponentId(usize);

impl ArchetypeComponentId {
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0
    }
}

impl SparseSetIndex for ArchetypeComponentId {
    #[inline]
    fn sparse_set_index(&self) -> usize {
        self.0
    }

    fn get_sparse_set_index(value: usize) -> Self {
        Self(value)
    }
}

pub struct Archetypes {
    pub(crate) archetypes: Vec<Archetype>,
    pub(crate) archetype_component_count: usize,
    archetype_ids: HashMap<ArchetypeIdentity, ArchetypeId>,
}

impl Default for Archetypes {
    fn default() -> Self {
        let mut archetypes = Archetypes {
            archetypes: Vec::new(),
            archetype_ids: Default::default(),
            archetype_component_count: 0,
        };
        archetypes.get_id_or_insert(TableId::empty(), Vec::new(), Vec::new());

        // adds the resource archetype. it is "special" in that it is inaccessible via a "hash",
        // which prevents entities from being added to it
        archetypes.archetypes.push(Archetype::new(
            ArchetypeId::resource(),
            TableId::empty(),
            Cow::Owned(Vec::new()),
            Cow::Owned(Vec::new()),
            Vec::new(),
            Vec::new(),
        ));
        archetypes
    }
}

impl Archetypes {
    #[inline]
    pub fn generation(&self) -> ArchetypeGeneration {
        ArchetypeGeneration(self.archetypes.len())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.archetypes.len()
    }

    #[inline]
    pub fn empty(&self) -> &Archetype {
        // SAFE: empty archetype always exists
        unsafe { self.archetypes.get_unchecked(ArchetypeId::empty().index()) }
    }

    #[inline]
    pub fn empty_mut(&mut self) -> &mut Archetype {
        // SAFE: empty archetype always exists
        unsafe {
            self.archetypes
                .get_unchecked_mut(ArchetypeId::empty().index())
        }
    }

    #[inline]
    pub fn resource(&self) -> &Archetype {
        // SAFE: resource archetype always exists
        unsafe {
            self.archetypes
                .get_unchecked(ArchetypeId::resource().index())
        }
    }

    #[inline]
    pub fn resource_mut(&mut self) -> &mut Archetype {
        // SAFE: resource archetype always exists
        unsafe {
            self.archetypes
                .get_unchecked_mut(ArchetypeId::resource().index())
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.archetypes.is_empty()
    }

    #[inline]
    pub fn get(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get(id.index())
    }

    #[inline]
    pub fn get_mut(&mut self, id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_mut(id.index())
    }

    #[inline]
    pub(crate) fn get_2_mut(
        &mut self,
        a: ArchetypeId,
        b: ArchetypeId,
    ) -> (&mut Archetype, &mut Archetype) {
        if a.0 == b.0 {
            panic!("both indexes were the same");
        }

        if a.index() > b.index() {
            let (b_slice, a_slice) = self.archetypes.split_at_mut(a.index());
            (&mut a_slice[0], &mut b_slice[b.index()])
        } else {
            let (a_slice, b_slice) = self.archetypes.split_at_mut(b.index());
            (&mut a_slice[a.index()], &mut b_slice[0])
        }
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter()
    }

    /// Gets the archetype id matching the given inputs or inserts a new one if it doesn't exist.
    /// `table_components` and `sparse_set_components` must be sorted
    ///
    /// # Safety
    /// TableId must exist in tables
    pub(crate) fn get_id_or_insert(
        &mut self,
        table_id: TableId,
        table_components: Vec<(EntityAtomKindId, Option<Entity>)>,
        sparse_set_components: Vec<(EntityAtomKindId, Option<Entity>)>,
    ) -> ArchetypeId {
        let table_components = Cow::from(table_components);
        let sparse_set_components = Cow::from(sparse_set_components);
        let archetype_identity = ArchetypeIdentity {
            sparse_set_components: sparse_set_components.clone(),
            table_components: table_components.clone(),
        };

        let archetypes = &mut self.archetypes;
        let archetype_component_count = &mut self.archetype_component_count;
        let mut next_archetype_component_id = move || {
            let id = ArchetypeComponentId(*archetype_component_count);
            *archetype_component_count += 1;
            id
        };
        *self
            .archetype_ids
            .entry(archetype_identity)
            .or_insert_with(move || {
                let id = ArchetypeId(archetypes.len());
                let table_archetype_components = (0..table_components.len())
                    .map(|_| next_archetype_component_id())
                    .collect();
                let sparse_set_archetype_components = (0..sparse_set_components.len())
                    .map(|_| next_archetype_component_id())
                    .collect();
                archetypes.push(Archetype::new(
                    id,
                    table_id,
                    table_components,
                    sparse_set_components,
                    table_archetype_components,
                    sparse_set_archetype_components,
                ));
                id
            })
    }

    #[inline]
    pub fn archetype_components_len(&self) -> usize {
        self.archetype_component_count
    }
}

impl Index<ArchetypeId> for Archetypes {
    type Output = Archetype;

    #[inline]
    fn index(&self, index: ArchetypeId) -> &Self::Output {
        &self.archetypes[index.index()]
    }
}

impl IndexMut<ArchetypeId> for Archetypes {
    #[inline]
    fn index_mut(&mut self, index: ArchetypeId) -> &mut Self::Output {
        &mut self.archetypes[index.index()]
    }
}
