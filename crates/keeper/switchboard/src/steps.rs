//! Unit tests of the steps of a turn as the Switchboard keeps and tells them: in order,
//! each with its session and its kind, and an edit with its path and lines (row SWB-04).
//! They go through [`crate::Switchboard::take`] and [`crate::Switchboard::session`], as
//! the Keeper does.
//!
//! Written by the Switchboard's package of slice 5.
