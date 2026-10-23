//! The controls more than one screen uses (plan 20261023_04, decisions 8
//! and 9): one component struct per file. They are candidates for
//! libcontrols, and stay in the app until moving one is decided.

pub mod control_row;
pub mod square_button;

pub use control_row::ControlRow;
pub use square_button::SquareButton;
