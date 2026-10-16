mod git;

use anyhow::Result;

// Temporary: dumps the log until the TUI lands in step 2.
fn main() -> Result<()> {
    let root = git::repo_root(&std::env::current_dir()?)?;
    for commit in git::log(&root)? {
        let commit = commit?;
        println!("{} {} — {} ({})", commit.short, commit.subject, commit.author, commit.date);
        for file in &commit.files {
            match &file.old_path {
                Some(old) => println!("    {} {old} → {}", file.status.letter(), file.path),
                None => println!("    {} {}", file.status.letter(), file.path),
            }
        }
    }
    Ok(())
}
