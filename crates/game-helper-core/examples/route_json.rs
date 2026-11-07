//! Read-only compatibility diagnostic: print the executable cache without saving it.
use anyhow::{Context, Result};
use game_helper_core::route::Route;
fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .context("usage: route_json <route.yaml>")?;
    let source = std::fs::read_to_string(&path).context("cannot read route")?;
    println!("{}", Route::parse(&source)?.cache_json()?);
    Ok(())
}
