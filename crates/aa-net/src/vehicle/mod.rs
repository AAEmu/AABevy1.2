//! Land-vehicle move codecs ported from the 1.2 `x2game.dll`.
//!
//! Phase A of the vehicle port: the pure move-struct codecs (`FUN_3961e2c0`,
//! `FUN_397ad5a0`, `FUN_397ad8c0`, `FUN_397ada60`, `FUN_397ad740`,
//! `FUN_397adc00`, `FUN_397ae020`, `FUN_397adca0`, `FUN_3938b460`), the whole
//! serializer (`FUN_397c0470`) and the move copy (`FUN_39024610`). The checked
//! CS089 type-2 request lives here too; driver/controller admission and the
//! driving simulation are not ported yet.

mod acosf;
mod asinf_tables;
pub mod codec;
pub mod control;
pub mod fixed;
pub mod move_unit;
mod sinf;

pub use codec::*;
pub use control::*;
pub use fixed::*;
pub use move_unit::*;

#[cfg(test)]
mod tests;
