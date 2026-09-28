//! Игровая логика без UI и GPU. Всё детерминировано и покрыто тестами.

pub mod board;
pub mod generator;
pub mod piece;
pub mod rng;
pub mod rules;

pub use board::{cell_color, cell_finish, cell_material, make_cell, make_cell_full, Board, Cell, Clear, SIZE};
pub use piece::{catalog, Family, Material, Piece, Shape, FINISH_COUNT};
pub use rng::Rng;
pub use rules::{Ability, AbilityOutcome, Difficulty, Game, Gain, Placement, Scoring, CELL_POINTS, MAX_ENERGY, PERFECT_BONUS};
