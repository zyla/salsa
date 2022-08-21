//! Basic test of accumulator functionality.

use crate::{
    cycle::CycleRecoveryStrategy,
    hash::FxDashMap,
    ingredient::{Ingredient, IngredientRequiresReset},
    key::DependencyIndex,
    runtime::local_state::QueryOrigin,
    storage::HasJar,
    DatabaseKeyIndex, Event, EventKind, IngredientIndex, Revision, Runtime,
};

pub trait Accumulator {
    type Data: Clone;
    type Jar;

    fn accumulator_ingredient<'db, Db>(db: &'db Db) -> &'db AccumulatorIngredient<Self::Data>
    where
        Db: ?Sized + HasJar<Self::Jar>;
}
pub struct AccumulatorIngredient<Data: Clone> {
    index: IngredientIndex,
    map: FxDashMap<DatabaseKeyIndex, AccumulatedValues<Data>>,
}

struct AccumulatedValues<Data> {
    produced_at: Revision,
    values: Vec<Data>,
}

impl<Data: Clone> AccumulatorIngredient<Data> {
    pub fn new(index: IngredientIndex) -> Self {
        Self {
            map: FxDashMap::default(),
            index,
        }
    }

    fn dependency_index(&self) -> DependencyIndex {
        DependencyIndex {
            ingredient_index: self.index,
            key_index: None,
        }
    }

    pub fn push(&self, runtime: &Runtime, value: Data) {
        let current_revision = runtime.current_revision();
        let (active_query, _) = match runtime.active_query() {
            Some(pair) => pair,
            None => {
                panic!("cannot accumulate values outside of an active query")
            }
        };

        let mut accumulated_values = self.map.entry(active_query).or_insert(AccumulatedValues {
            values: vec![],
            produced_at: current_revision,
        });

        // This is the first push in a new revision. Reset.
        if accumulated_values.produced_at != current_revision {
            accumulated_values.values.truncate(0);
            accumulated_values.produced_at = current_revision;
        }

        runtime.add_output(self.dependency_index());
        accumulated_values.values.push(value);
    }

    pub(crate) fn produced_by(
        &self,
        runtime: &Runtime,
        query: DatabaseKeyIndex,
        output: &mut Vec<Data>,
    ) {
        let current_revision = runtime.current_revision();
        if let Some(v) = self.map.get(&query) {
            // FIXME: We don't currently have a good way to identify the value that was read.
            // You can't report is as a tracked read of `query`, because the return value of query is not being read here --
            // instead it is the set of values accumuated by `query`.
            runtime.report_untracked_read();

            let AccumulatedValues {
                values,
                produced_at,
            } = v.value();

            if *produced_at == current_revision {
                output.extend(values.iter().cloned());
            }
        }
    }
}

impl<DB: ?Sized, Data> Ingredient<DB> for AccumulatorIngredient<Data>
where
    DB: crate::Database,
    Data: Clone,
{
    fn maybe_changed_after(&self, _db: &DB, _input: DependencyIndex, _revision: Revision) -> bool {
        panic!("nothing should ever depend on an accumulator directly")
    }

    fn cycle_recovery_strategy(&self) -> CycleRecoveryStrategy {
        CycleRecoveryStrategy::Panic
    }

    fn origin(&self, _key_index: crate::Id) -> Option<QueryOrigin> {
        None
    }

    fn mark_validated_output(
        &self,
        db: &DB,
        executor: DatabaseKeyIndex,
        output_key: Option<crate::Id>,
    ) {
        assert!(output_key.is_none());
        let current_revision = db.salsa_runtime().current_revision();
        if let Some(mut v) = self.map.get_mut(&executor) {
            // The value is still valid in the new revision.
            v.produced_at = current_revision;
        }
    }

    fn remove_stale_output(
        &self,
        db: &DB,
        executor: DatabaseKeyIndex,
        stale_output_key: Option<crate::Id>,
    ) {
        assert!(stale_output_key.is_none());
        if self.map.remove(&executor).is_some() {
            db.salsa_event(Event {
                runtime_id: db.salsa_runtime().id(),
                kind: EventKind::DidDiscardAccumulated {
                    executor_key: executor,
                    accumulator: self.dependency_index(),
                },
            })
        }
    }

    fn reset_for_new_revision(&mut self) {
        panic!("unexpected reset on accumulator")
    }

    fn salsa_struct_deleted(&self, _db: &DB, _id: crate::Id) {
        panic!("unexpected call: accumulator is not registered as a dependent fn");
    }

    fn fmt_index(
        &self,
        _index: Option<crate::Id>,
        _fmt: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        todo!()
    }
}

impl<Data> IngredientRequiresReset for AccumulatorIngredient<Data>
where
    Data: Clone,
{
    const RESET_ON_NEW_REVISION: bool = false;
}
