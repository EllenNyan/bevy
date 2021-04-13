use crate::{
    archetype::ArchetypeId,
    component::{ComponentTicks, RelationKindId, RelationshipKindInfo, Relationships},
    entity::Entity,
    storage::{BlobVec, SparseSet},
};
use bevy_utils::{AHasher, HashMap, StableHashMap};
use std::{
    cell::UnsafeCell,
    hash::{Hash, Hasher},
    ops::{Index, IndexMut},
    ptr::NonNull,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableId(usize);

impl TableId {
    #[inline]
    pub fn new(index: usize) -> Self {
        TableId(index)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0
    }

    #[inline]
    pub const fn empty() -> TableId {
        TableId(0)
    }
}

pub struct Column {
    pub(crate) relationship: (RelationKindId, Option<Entity>),
    pub(crate) data: BlobVec,
    pub(crate) ticks: UnsafeCell<Vec<ComponentTicks>>,
}

impl Column {
    #[inline]
    pub fn with_capacity(
        relation_kind: &RelationshipKindInfo,
        target: Option<Entity>,
        capacity: usize,
    ) -> Self {
        Column {
            relationship: (relation_kind.id(), target),
            data: BlobVec::new(
                relation_kind.data_layout().layout(),
                relation_kind.data_layout().drop(),
                capacity,
            ),
            ticks: UnsafeCell::new(Vec::with_capacity(capacity)),
        }
    }

    /// # Safety
    /// Assumes data has already been allocated for the given row/column.
    /// Allows aliased mutable accesses to the data at the given `row`. Caller must ensure that this
    /// does not happen.
    #[inline]
    pub unsafe fn set_unchecked(&self, row: usize, data: *mut u8) {
        self.data.set_unchecked(row, data);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// # Safety
    /// Assumes data has already been allocated for the given row/column.
    /// Allows aliased mutable accesses to the row's [ComponentTicks].
    /// Caller must ensure that this does not happen.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_ticks_unchecked_mut(&self, row: usize) -> &mut ComponentTicks {
        debug_assert!(row < self.len());
        (*self.ticks.get()).get_unchecked_mut(row)
    }

    #[inline]
    pub(crate) unsafe fn swap_remove_unchecked(&mut self, row: usize) {
        self.data.swap_remove_and_drop_unchecked(row);
        (*self.ticks.get()).swap_remove(row);
    }

    #[inline]
    pub(crate) unsafe fn swap_remove_and_forget_unchecked(
        &mut self,
        row: usize,
    ) -> (*mut u8, ComponentTicks) {
        let data = self.data.swap_remove_and_forget_unchecked(row);
        let ticks = (*self.ticks.get()).swap_remove(row);
        (data, ticks)
    }

    /// # Safety
    /// allocated value must be immediately set at the returned row
    pub(crate) unsafe fn push_uninit(&mut self) -> usize {
        let row = self.data.push_uninit();
        (*self.ticks.get()).push(ComponentTicks::new(0));
        row
    }

    #[inline]
    pub(crate) fn reserve(&mut self, additional: usize) {
        self.data.reserve(additional);
        // SAFE: unique access to self
        unsafe {
            let ticks = &mut (*self.ticks.get());
            ticks.reserve(additional);
        }
    }

    /// # Safety
    /// must ensure rust mutability rules are not violated
    #[inline]
    pub unsafe fn get_ptr(&self) -> NonNull<u8> {
        self.data.get_ptr()
    }

    /// # Safety
    /// must ensure rust mutability rules are not violated
    #[inline]
    pub unsafe fn get_ticks_mut_ptr(&self) -> *mut ComponentTicks {
        (*self.ticks.get()).as_mut_ptr()
    }

    /// # Safety
    /// must ensure rust mutability rules are not violated
    #[inline]
    pub unsafe fn get_unchecked(&self, row: usize) -> *mut u8 {
        debug_assert!(row < self.data.len());
        self.data.get_unchecked(row)
    }

    /// # Safety
    /// must ensure rust mutability rules are not violated
    #[inline]
    pub unsafe fn get_ticks_unchecked(&self, row: usize) -> *mut ComponentTicks {
        debug_assert!(row < (*self.ticks.get()).len());
        self.get_ticks_mut_ptr().add(row)
    }

    #[inline]
    pub(crate) fn check_change_ticks(&mut self, change_tick: u32) {
        let ticks = unsafe { (*self.ticks.get()).iter_mut() };
        for component_ticks in ticks {
            component_ticks.check_ticks(change_tick);
        }
    }
}

pub struct ColIter<'a> {
    no_target_col: Option<&'a Column>,
    target_cols: std::collections::hash_map::Iter<'a, Entity, Column>,
}

impl<'a> Iterator for ColIter<'a> {
    type Item = (Option<Entity>, &'a Column);

    fn next(&mut self) -> Option<Self::Item> {
        match self.target_cols.next() {
            Some((e, col)) => Some((Some(*e), col)),
            None => self.no_target_col.take().map(|col| (None, col)),
        }
    }
}

pub struct Table {
    pub(crate) columns: SparseSet<RelationKindId, (Option<Column>, StableHashMap<Entity, Column>)>,
    entities: Vec<Entity>,
    archetypes: Vec<ArchetypeId>,
    grow_amount: usize,
    capacity: usize,
}

impl Table {
    pub const fn new(grow_amount: usize) -> Table {
        Self {
            columns: SparseSet::new(),
            entities: Vec::new(),
            archetypes: Vec::new(),
            grow_amount,
            capacity: 0,
        }
    }

    pub fn with_capacity(capacity: usize, column_capacity: usize, grow_amount: usize) -> Table {
        Self {
            columns: SparseSet::with_capacity(column_capacity),
            entities: Vec::with_capacity(capacity),
            archetypes: Vec::new(),
            grow_amount,
            capacity,
        }
    }

    pub fn columns_of_kind(&self, kind: RelationKindId) -> Option<ColIter> {
        let columns = self.columns.get(kind)?;
        Some(ColIter {
            no_target_col: columns.0.as_ref(),
            target_cols: columns.1.iter(),
        })
    }

    pub fn columns(&self) -> impl Iterator<Item = &Column> {
        self.columns
            .values()
            .map(|columns| columns.1.values().chain(columns.0.as_ref()))
            .flatten()
    }

    fn columns_mut(&mut self) -> impl Iterator<Item = &mut Column> {
        self.columns
            .values_mut()
            .map(|columns| columns.1.values_mut().chain(columns.0.as_mut()))
            .flatten()
    }

    #[inline]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn add_archetype(&mut self, archetype_id: ArchetypeId) {
        self.archetypes.push(archetype_id);
    }

    pub fn add_column(&mut self, component_kind: &RelationshipKindInfo, target: Option<Entity>) {
        let column = self
            .columns
            .get_or_insert_with(component_kind.id(), || (None, StableHashMap::default()));

        match target {
            Some(target) => {
                column.1.insert(
                    target,
                    Column::with_capacity(component_kind, Some(target), self.capacity),
                );
            }
            None => {
                column.0 = Some(Column::with_capacity(component_kind, None, self.capacity));
            }
        }
    }

    /// Removes the entity at the given row and returns the entity swapped in to replace it (if an
    /// entity was swapped in)
    ///
    /// # Safety
    /// `row` must be in-bounds
    pub unsafe fn swap_remove_unchecked(&mut self, row: usize) -> Option<Entity> {
        for column in self.columns_mut() {
            column.swap_remove_unchecked(row);
        }
        let is_last = row == self.entities.len() - 1;
        self.entities.swap_remove(row);
        if is_last {
            None
        } else {
            Some(self.entities[row])
        }
    }

    /// Moves the `row` column values to `new_table`, for the columns shared between both tables.
    /// Returns the index of the new row in `new_table` and the entity in this table swapped in
    /// to replace it (if an entity was swapped in). missing columns will be "forgotten". It is
    /// the caller's responsibility to drop them
    ///
    /// # Safety
    /// Row must be in-bounds
    pub unsafe fn move_to_and_forget_missing_unchecked(
        &mut self,
        row: usize,
        new_table: &mut Table,
    ) -> TableMoveResult {
        debug_assert!(row < self.len());
        let is_last = row == self.entities.len() - 1;
        let new_row = new_table.allocate(self.entities.swap_remove(row));
        for column in self.columns_mut() {
            let (data, ticks) = column.swap_remove_and_forget_unchecked(row);
            if let Some(new_column) =
                new_table.get_column_mut(column.relationship.0, column.relationship.1)
            {
                new_column.set_unchecked(new_row, data);
                *new_column.get_ticks_unchecked_mut(new_row) = ticks;
            }
        }
        TableMoveResult {
            new_row,
            swapped_entity: if is_last {
                None
            } else {
                Some(self.entities[row])
            },
        }
    }

    /// Moves the `row` column values to `new_table`, for the columns shared between both tables.
    /// Returns the index of the new row in `new_table` and the entity in this table swapped in
    /// to replace it (if an entity was swapped in).
    ///
    /// # Safety
    /// row must be in-bounds
    pub unsafe fn move_to_and_drop_missing_unchecked(
        &mut self,
        row: usize,
        new_table: &mut Table,
    ) -> TableMoveResult {
        debug_assert!(row < self.len());
        let is_last = row == self.entities.len() - 1;
        let new_row = new_table.allocate(self.entities.swap_remove(row));
        for column in self.columns_mut() {
            if let Some(new_column) =
                new_table.get_column_mut(column.relationship.0, column.relationship.1)
            {
                let (data, ticks) = column.swap_remove_and_forget_unchecked(row);
                new_column.set_unchecked(new_row, data);
                *new_column.get_ticks_unchecked_mut(new_row) = ticks;
            } else {
                column.swap_remove_unchecked(row);
            }
        }
        TableMoveResult {
            new_row,
            swapped_entity: if is_last {
                None
            } else {
                Some(self.entities[row])
            },
        }
    }

    /// Moves the `row` column values to `new_table`, for the columns shared between both tables.
    /// Returns the index of the new row in `new_table` and the entity in this table swapped in
    /// to replace it (if an entity was swapped in).
    ///
    /// # Safety
    /// `row` must be in-bounds. `new_table` must contain every component this table has
    pub unsafe fn move_to_superset_unchecked(
        &mut self,
        row: usize,
        new_table: &mut Table,
    ) -> TableMoveResult {
        debug_assert!(row < self.len());
        let is_last = row == self.entities.len() - 1;
        let new_row = new_table.allocate(self.entities.swap_remove(row));
        for column in self.columns_mut() {
            let new_column = new_table
                .get_column_mut(column.relationship.0, column.relationship.1)
                .unwrap();
            let (data, ticks) = column.swap_remove_and_forget_unchecked(row);
            new_column.set_unchecked(new_row, data);
            *new_column.get_ticks_unchecked_mut(new_row) = ticks;
        }
        TableMoveResult {
            new_row,
            swapped_entity: if is_last {
                None
            } else {
                Some(self.entities[row])
            },
        }
    }

    #[inline]
    pub fn get_column(
        &self,
        component_id: RelationKindId,
        target: Option<Entity>,
    ) -> Option<&Column> {
        let col = self.columns.get(component_id)?;
        match target {
            Some(target) => col.1.get(&target),
            None => col.0.as_ref(),
        }
    }

    #[inline]
    pub fn get_column_mut(
        &mut self,
        component_id: RelationKindId,
        target: Option<Entity>,
    ) -> Option<&mut Column> {
        let col = self.columns.get_mut(component_id)?;
        match target {
            Some(target) => col.1.get_mut(&target),
            None => col.0.as_mut(),
        }
    }

    #[inline]
    pub fn has_column(&self, component_id: RelationKindId, target: Option<Entity>) -> bool {
        self.columns
            .get(component_id)
            .and_then(|col| match target {
                None => col.0.as_ref(),
                Some(target) => col.1.get(&target),
            })
            .is_some()
    }

    pub fn reserve(&mut self, amount: usize) {
        let available_space = self.capacity - self.len();
        if available_space < amount {
            let min_capacity = self.len() + amount;
            // normally we would check if min_capacity is 0 for the below calculation, but amount >
            // available_space and available_space > 0, so min_capacity > 1
            let new_capacity =
                ((min_capacity + self.grow_amount - 1) / self.grow_amount) * self.grow_amount;
            let reserve_amount = new_capacity - self.len();
            for column in self.columns_mut() {
                column.reserve(reserve_amount);
            }
            self.entities.reserve(reserve_amount);
            self.capacity = new_capacity;
        }
    }

    /// Allocates space for a new entity
    ///
    /// # Safety
    /// the allocated row must be written to immediately with valid values in each column
    pub unsafe fn allocate(&mut self, entity: Entity) -> usize {
        self.reserve(1);
        let index = self.entities.len();
        self.entities.push(entity);
        for column in self.columns_mut() {
            column.data.set_len(index + 1);
            (*column.ticks.get()).push(ComponentTicks::new(0));
        }
        index
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub(crate) fn check_change_ticks(&mut self, change_tick: u32) {
        for column in self.columns_mut() {
            column.check_change_ticks(change_tick);
        }
    }
}

pub struct Tables {
    tables: Vec<Table>,
    table_ids: HashMap<u64, TableId>,
}

impl Default for Tables {
    fn default() -> Self {
        let empty_table = Table::with_capacity(0, 0, 64);
        Tables {
            tables: vec![empty_table],
            table_ids: HashMap::default(),
        }
    }
}

pub struct TableMoveResult {
    pub swapped_entity: Option<Entity>,
    pub new_row: usize,
}

impl Tables {
    #[inline]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    #[inline]
    pub fn get(&self, id: TableId) -> Option<&Table> {
        self.tables.get(id.index())
    }

    #[inline]
    pub fn get_mut(&mut self, id: TableId) -> Option<&mut Table> {
        self.tables.get_mut(id.index())
    }

    #[inline]
    pub(crate) fn get_2_mut(&mut self, a: TableId, b: TableId) -> (&mut Table, &mut Table) {
        if a.index() > b.index() {
            let (b_slice, a_slice) = self.tables.split_at_mut(a.index());
            (&mut a_slice[0], &mut b_slice[b.index()])
        } else {
            let (a_slice, b_slice) = self.tables.split_at_mut(b.index());
            (&mut a_slice[a.index()], &mut b_slice[0])
        }
    }

    /// # Safety
    /// `component_ids` must contain components that exist in `components`
    pub unsafe fn get_id_or_insert(
        &mut self,
        component_ids: &[(RelationKindId, Option<Entity>)],
        components: &Relationships,
    ) -> TableId {
        let mut hasher = AHasher::default();
        component_ids.hash(&mut hasher);
        let hash = hasher.finish();
        let tables = &mut self.tables;
        *self.table_ids.entry(hash).or_insert_with(move || {
            let mut table = Table::with_capacity(0, component_ids.len(), 64);
            for component_id in component_ids.iter() {
                table.add_column(
                    components.get_relation_kind(component_id.0).unwrap(),
                    component_id.1,
                );
            }
            tables.push(table);
            TableId(tables.len() - 1)
        })
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Table> {
        self.tables.iter()
    }

    pub(crate) fn check_change_ticks(&mut self, change_tick: u32) {
        for table in self.tables.iter_mut() {
            table.check_change_ticks(change_tick);
        }
    }
}

impl Index<TableId> for Tables {
    type Output = Table;

    #[inline]
    fn index(&self, index: TableId) -> &Self::Output {
        &self.tables[index.index()]
    }
}

impl IndexMut<TableId> for Tables {
    #[inline]
    fn index_mut(&mut self, index: TableId) -> &mut Self::Output {
        &mut self.tables[index.index()]
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        component::{ComponentDescriptor, Relationships, StorageType, TypeInfo},
        entity::Entity,
        storage::Table,
    };

    #[test]
    fn table() {
        let mut components = Relationships::default();
        let type_info = TypeInfo::of::<usize>();
        let component_id = components.get_component_kind_or_insert(
            type_info.type_id(),
            ComponentDescriptor::from_generic::<usize>(StorageType::Table),
        );
        let mut table = Table::with_capacity(0, 1, 64);
        table.add_column(component_id, None);
        let entities = (0..200).map(Entity::new).collect::<Vec<_>>();
        for (row, entity) in entities.iter().cloned().enumerate() {
            unsafe {
                table.allocate(entity);
                let mut value = row;
                let value_ptr = ((&mut value) as *mut usize).cast::<u8>();
                table
                    .get_column(component_id.id(), None)
                    .unwrap()
                    .set_unchecked(row, value_ptr);
            };
        }

        assert_eq!(table.capacity(), 256);
        assert_eq!(table.len(), 200);
    }
}
