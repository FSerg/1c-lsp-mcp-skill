use include_dir::{include_dir, Dir};

pub static FRONTEND_DIR: Dir<'_> = include_dir!("$OUT_DIR/frontend-dist");
