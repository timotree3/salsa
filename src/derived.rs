use crate::debug::TableEntry;
use crate::durability::Durability;
use crate::lru::Lru;
use crate::plumbing::DerivedQueryStorageOps;
use crate::plumbing::GlobalQueryStorageOps;
use crate::plumbing::LocalQueryStorageOps;
use crate::plumbing::LruQueryStorageOps;
use crate::plumbing::QueryFunction;
use crate::plumbing::QueryStorageMassOps;
use crate::runtime::{FxIndexMap, StampedValue};
use crate::{Database, DatabaseKeyIndex, QueryDb, Revision};
use parking_lot::RwLock;
use std::borrow::Borrow;
use std::convert::TryFrom;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

mod slot;
use slot::Slot;

/// Memoized queries store the result plus a list of the other queries
/// that they invoked. This means we can avoid recomputing them when
/// none of those inputs have changed.
pub type MemoizedStorage<Q> = DerivedStorage<Q, AlwaysMemoizeValue>;

/// Global storage for memoized queries.
pub type MemoizedGlobalStorage<Q> = DerivedGlobalStorage<Q, AlwaysMemoizeValue>;

/// "Dependency" queries just track their dependencies and not the
/// actual value (which they produce on demand). This lessens the
/// storage requirements.
pub type DependencyStorage<Q> = DerivedStorage<Q, NeverMemoizeValue>;

/// Global storage for dependency queries.
pub type DependencyGlobalStorage<Q> = DerivedGlobalStorage<Q, NeverMemoizeValue>;

/// Handles storage where the value is 'derived' by executing a
/// function (in contrast to "inputs").
pub struct DerivedStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
{
    /// There is no data here yet -- but there will be!
    policy: PhantomData<(Q, MP)>,
}

pub struct DerivedGlobalStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
{
    group_index: u16,
    lru_list: Lru<Slot<Q, MP>>,
    slot_map: RwLock<FxIndexMap<Q::Key, Arc<Slot<Q, MP>>>>,

    policy: PhantomData<(Q, MP)>,
}

impl<Q, MP> std::panic::RefUnwindSafe for DerivedStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
    Q::Key: std::panic::RefUnwindSafe,
    Q::Value: std::panic::RefUnwindSafe,
{
}

impl<Q, MP> std::panic::RefUnwindSafe for DerivedGlobalStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
    Q::Key: std::panic::RefUnwindSafe,
    Q::Value: std::panic::RefUnwindSafe,
{
}

pub trait MemoizationPolicy<Q>: Send + Sync
where
    Q: QueryFunction,
{
    fn should_memoize_value(key: &Q::Key) -> bool;

    fn memoized_value_eq(old_value: &Q::Value, new_value: &Q::Value) -> bool;
}

pub enum AlwaysMemoizeValue {}
impl<Q> MemoizationPolicy<Q> for AlwaysMemoizeValue
where
    Q: QueryFunction,
    Q::Value: Eq,
{
    fn should_memoize_value(_key: &Q::Key) -> bool {
        true
    }

    fn memoized_value_eq(old_value: &Q::Value, new_value: &Q::Value) -> bool {
        old_value == new_value
    }
}

pub enum NeverMemoizeValue {}
impl<Q> MemoizationPolicy<Q> for NeverMemoizeValue
where
    Q: QueryFunction,
{
    fn should_memoize_value(_key: &Q::Key) -> bool {
        false
    }

    fn memoized_value_eq(_old_value: &Q::Value, _new_value: &Q::Value) -> bool {
        panic!("cannot reach since we never memoize")
    }
}

impl<Q, MP> LocalQueryStorageOps<Q> for DerivedStorage<Q, MP>
where
    Q: QueryFunction<GlobalStorage = DerivedGlobalStorage<Q, MP>>,
    MP: MemoizationPolicy<Q>,
{
    const CYCLE_STRATEGY: crate::plumbing::CycleRecoveryStrategy = Q::CYCLE_STRATEGY;

    fn new(_group_index: u16) -> Self {
        DerivedStorage {
            policy: PhantomData,
        }
    }

    fn fmt_index(
        &self,
        db: &<Q as QueryDb<'_>>::DynDb,
        index: DatabaseKeyIndex,
        fmt: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        crate::plumbing::global_query_storage::<Q>(db).fmt_index(db, index, fmt)
    }

    fn maybe_changed_since(
        &self,
        db: &<Q as QueryDb<'_>>::DynDb,
        input: DatabaseKeyIndex,
        revision: Revision,
    ) -> bool {
        crate::plumbing::global_query_storage::<Q>(db).maybe_changed_since(db, input, revision)
    }

    fn fetch(&self, db: &<Q as QueryDb<'_>>::DynDb, key: &Q::Key) -> Q::Value {
        crate::plumbing::global_query_storage::<Q>(db).fetch(db, key)
    }

    fn durability(&self, db: &<Q as QueryDb<'_>>::DynDb, key: &Q::Key) -> Durability {
        crate::plumbing::global_query_storage::<Q>(db).durability(db, key)
    }

    fn entries<C>(&self, db: &<Q as QueryDb<'_>>::DynDb) -> C
    where
        C: std::iter::FromIterator<TableEntry<Q::Key, Q::Value>>,
    {
        crate::plumbing::global_query_storage::<Q>(db).entries(db)
    }
}

impl<Q, MP> QueryStorageMassOps for DerivedStorage<Q, MP>
where
    Q: QueryFunction<GlobalStorage = DerivedGlobalStorage<Q, MP>>,
    MP: MemoizationPolicy<Q>,
{
    fn purge(&self) {}
}

impl<Q, MP> LruQueryStorageOps<Q> for DerivedStorage<Q, MP>
where
    Q: QueryFunction<GlobalStorage = DerivedGlobalStorage<Q, MP>>,
    MP: MemoizationPolicy<Q>,
{
    fn set_lru_capacity(&self, db: &<Q as QueryDb<'_>>::DynDb, new_capacity: usize) {
        crate::plumbing::global_query_storage::<Q>(db).set_lru_capacity(db, new_capacity)
    }
}

impl<Q, MP> DerivedQueryStorageOps<Q> for DerivedStorage<Q, MP>
where
    Q: QueryFunction<GlobalStorage = DerivedGlobalStorage<Q, MP>>,
    MP: MemoizationPolicy<Q>,
{
    fn invalidate<S>(&self, db: &mut <Q as QueryDb<'_>>::DynDb, key: &S)
    where
        S: Eq + Hash,
        Q::Key: Borrow<S>,
    {
        let global_storage = crate::plumbing::global_query_storage::<Q>(db).clone();
        global_storage.invalidate(db, key)
    }
}

impl<Q, MP> GlobalQueryStorageOps<Q> for DerivedGlobalStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
{
    fn new(group_index: u16) -> Self {
        DerivedGlobalStorage {
            group_index,
            slot_map: RwLock::new(FxIndexMap::default()),
            lru_list: Default::default(),
            policy: PhantomData,
        }
    }
}

impl<Q, MP> QueryStorageMassOps for DerivedGlobalStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
{
    fn purge(&self) {
        self.lru_list.purge();
        *self.slot_map.write() = Default::default();
    }
}

impl<Q, MP> DerivedGlobalStorage<Q, MP>
where
    Q: QueryFunction,
    MP: MemoizationPolicy<Q>,
{
    fn slot(&self, key: &Q::Key) -> Arc<Slot<Q, MP>> {
        if let Some(v) = self.slot_map.read().get(key) {
            return v.clone();
        }

        let mut write = self.slot_map.write();
        let entry = write.entry(key.clone());
        let key_index = u32::try_from(entry.index()).unwrap();
        let database_key_index = DatabaseKeyIndex {
            group_index: self.group_index,
            query_index: Q::QUERY_INDEX,
            key_index,
        };
        entry
            .or_insert_with(|| Arc::new(Slot::new(key.clone(), database_key_index)))
            .clone()
    }

    fn fmt_index(
        &self,
        _db: &<Q as QueryDb<'_>>::DynDb,
        index: DatabaseKeyIndex,
        fmt: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        assert_eq!(index.group_index, self.group_index);
        assert_eq!(index.query_index, Q::QUERY_INDEX);
        let slot_map = self.slot_map.read();
        let key = slot_map.get_index(index.key_index as usize).unwrap().0;
        write!(fmt, "{}({:?})", Q::QUERY_NAME, key)
    }

    fn maybe_changed_since(
        &self,
        db: &<Q as QueryDb<'_>>::DynDb,
        input: DatabaseKeyIndex,
        revision: Revision,
    ) -> bool {
        assert_eq!(input.group_index, self.group_index);
        assert_eq!(input.query_index, Q::QUERY_INDEX);
        let slot = self
            .slot_map
            .read()
            .get_index(input.key_index as usize)
            .unwrap()
            .1
            .clone();
        slot.maybe_changed_since(db, revision)
    }

    fn fetch(&self, db: &<Q as QueryDb<'_>>::DynDb, key: &Q::Key) -> Q::Value {
        db.unwind_if_cancelled();

        let slot = self.slot(key);
        let StampedValue {
            value,
            durability,
            changed_at,
        } = slot.read(db);

        if let Some(evicted) = self.lru_list.record_use(&slot) {
            evicted.evict();
        }

        db.salsa_runtime()
            .report_query_read_and_panic_if_cycle_resulted(
                slot.database_key_index(),
                durability,
                changed_at,
            );

        value
    }

    fn durability(&self, db: &<Q as QueryDb<'_>>::DynDb, key: &Q::Key) -> Durability {
        self.slot(key).durability(db)
    }

    fn entries<C>(&self, _db: &<Q as QueryDb<'_>>::DynDb) -> C
    where
        C: std::iter::FromIterator<TableEntry<Q::Key, Q::Value>>,
    {
        let slot_map = self.slot_map.read();
        slot_map
            .values()
            .filter_map(|slot| slot.as_table_entry())
            .collect()
    }

    fn invalidate<S>(&self, db: &mut <Q as QueryDb<'_>>::DynDb, key: &S)
    where
        S: Eq + Hash,
        Q::Key: Borrow<S>,
    {
        db.salsa_runtime_mut()
            .with_incremented_revision(&mut |new_revision| {
                let map_read = self.slot_map.read();

                if let Some(slot) = map_read.get(key) {
                    if let Some(durability) = slot.invalidate(new_revision) {
                        return Some(durability);
                    }
                }

                None
            })
    }

    fn set_lru_capacity(&self, _db: &<Q as QueryDb<'_>>::DynDb, new_capacity: usize) {
        self.lru_list.set_lru_capacity(new_capacity);
    }
}
