use super::{DatabaseKeyIndex, Revision};

pub trait Ingredient {
    fn maybe_changed_after(&self, input: DatabaseKeyIndex, revision: Revision) -> bool;
}

/// Optional trait for ingredients that wish to be notified when new revisions are
/// about to occur. If ingredients wish to receive these method calls,
/// they need to indicate that by invoking [`Ingredients::push_mut`] during initialization.
pub trait MutIngredient: Ingredient {
    /// Invoked when a new revision is about to start. This gives ingredients
    /// a chance to flush data and so forth.
    fn reset_for_new_revision(&mut self);
}
