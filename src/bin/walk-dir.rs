use oneway::tree::find_files;
use std::io;

fn main() -> io::Result<()> {
    env_logger::init();

    let path = std::env::args()
        .skip(1)
        .next()
        .unwrap_or_else(|| String::from("."));
    let path = std::path::PathBuf::from(path);
    println!("path = {}", path.display());

    let entries = find_files(path, true, |_p| {
        return true;
    });
    for entry in &entries {
        println!("{}", entry.display());
    }

    Ok(())
}
