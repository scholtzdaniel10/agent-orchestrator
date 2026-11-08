//! The keyboard UI. Ratatui draws [`view::screen_model`]; [`state::reduce`] owns the keys.

mod render;
mod session;
mod state;
mod view;

pub use session::open;

#[cfg(test)]
mod tests;
