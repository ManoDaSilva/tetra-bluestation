//! BS T1 test mode: a minimal replacement for the signalling stack, used for hardware validation.
//! Enabled with `stack_mode = "BsT1"`.

pub mod t1_entity;
pub use t1_entity::T1TestBs;

pub mod dl_gen;
pub mod prbs;
