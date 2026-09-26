pub mod about;
pub mod chart;
pub mod files;
pub mod toolbar;
pub mod tree;

/// What the left panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SideView {
    #[default]
    Folders,
    LargestFiles,
}
