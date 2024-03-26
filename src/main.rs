use git2::Repository;
use home::home_dir;
use std::{
    fs::File,
    process::{exit, Command},
};

const URL: &str = "https://github.com/CobbCoding1/Cano.git";
const INSTALL_DIR: &str = "/usr/bin/";
const INSTALL_FILE_PATH: &str = "/usr/bin/cano";

/**
 *  NOTE: The following three methods are from an git2-rs example (https://github.com/rust-lang/git2-rs/blob/master/examples/pull.rs)
 */

fn fast_forward(
    repo: &Repository,
    lb: &mut git2::Reference,
    rc: &git2::AnnotatedCommit,
) -> Result<(), git2::Error> {
    let name = match lb.name() {
        Some(s) => s.to_string(),
        None => String::from_utf8_lossy(lb.name_bytes()).to_string(),
    };
    let msg = format!("Fast-Forward: Setting {} to id: {}", name, rc.id());
    println!("{}", msg);
    lb.set_target(rc.id(), &msg)?;
    repo.set_head(&name)?;
    repo.checkout_head(Some(
        git2::build::CheckoutBuilder::default()
            // For some reason the force is required to make the working directory actually get updated
            // I suspect we should be adding some logic to handle dirty working directory states
            // but this is just an example so maybe not.
            .force(),
    ))?;
    Ok(())
}

fn normal_merge(
    repo: &Repository,
    local: &git2::AnnotatedCommit,
    remote: &git2::AnnotatedCommit,
) -> Result<(), git2::Error> {
    let local_tree = repo.find_commit(local.id())?.tree()?;
    let remote_tree = repo.find_commit(remote.id())?.tree()?;
    let ancestor = repo
        .find_commit(repo.merge_base(local.id(), remote.id())?)?
        .tree()?;
    let mut idx = repo.merge_trees(&ancestor, &local_tree, &remote_tree, None)?;

    if idx.has_conflicts() {
        repo.checkout_index(Some(&mut idx), None)?;
        return Ok(());
    }
    let result_tree = repo.find_tree(idx.write_tree_to(repo)?)?;
    // now create the merge commit
    let msg = format!("Merge: {} into {}", remote.id(), local.id());
    let sig = repo.signature()?;
    let local_commit = repo.find_commit(local.id())?;
    let remote_commit = repo.find_commit(remote.id())?;
    // Do our merge commit and set current branch head to that commit.
    let _merge_commit = repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &msg,
        &result_tree,
        &[&local_commit, &remote_commit],
    )?;
    // Set working tree to match head.
    repo.checkout_head(None)?;
    Ok(())
}

fn do_merge<'a>(
    repo: &'a Repository,
    remote_branch: &str,
    fetch_commit: git2::AnnotatedCommit<'a>,
) -> Result<(), git2::Error> {
    // 1. do a merge analysis
    let analysis = repo.merge_analysis(&[&fetch_commit])?;

    // 2. Do the appropriate merge
    if analysis.0.is_fast_forward() {
        // do a fast forward
        let refname = format!("refs/heads/{}", remote_branch);
        match repo.find_reference(&refname) {
            Ok(mut r) => {
                fast_forward(repo, &mut r, &fetch_commit)?;
            }
            Err(_) => {
                // The branch doesn't exist so just set the reference to the
                // commit directly. Usually this is because you are pulling
                // into an empty repository.
                repo.reference(
                    &refname,
                    fetch_commit.id(),
                    true,
                    &format!("Setting {} to {}", remote_branch, fetch_commit.id()),
                )?;
                repo.set_head(&refname)?;
                repo.checkout_head(Some(
                    git2::build::CheckoutBuilder::default()
                        .allow_conflicts(true)
                        .conflict_style_merge(true)
                        .force(),
                ))?;
            }
        };
    } else if analysis.0.is_normal() {
        // do a normal merge
        let head_commit = repo.reference_to_annotated_commit(&repo.head()?)?;
        normal_merge(&repo, &head_commit, &fetch_commit)?;
    } else {
        println!("Nothing to do...");
    }
    Ok(())
}

fn build_install(path: &str) {
    println!("Building cano...");
    let build_success = Command::new("make")
        .arg("-B")
        .current_dir(path)
        .output()
        .expect("Failed to run gnu-make.");
    print!("{}", String::from_utf8_lossy(&build_success.stdout));
    if build_success.status.success() {
        println!("Build successful.")
    } else {
        eprintln!("Build failed.");
        exit(1);
    }

    let cano_bin = format!("{path}build/cano");
    let copy_success = Command::new("sudo")
        .arg("install")
        .arg("-v")
        .arg(cano_bin)
        .arg(INSTALL_DIR)
        .output()
        .expect("Failed to copy cano binary.");
    print!("{}", String::from_utf8_lossy(&copy_success.stdout));
    if copy_success.status.success() {
        println!("Install successful.")
    } else {
        eprintln!("Install failed. (canoup needs root permissions)");
        exit(1);
    }
}

fn main() {
    let home_dir = match home_dir() {
        Some(d) => d,
        None => {
            eprintln!("Failed to find home directory.");
            exit(1);
        }
    };
    let home_dir_str = match home_dir.to_str() {
        Some(s) => s,
        None => {
            eprintln!("Failed to convert home directory to string.");
            exit(1);
        }
    };
    let cano_dir = format!("{home_dir_str}/cano/");

    let mut cloned = false;
    let repo = match Repository::open(cano_dir.clone()) {
        Ok(repo) => repo,
        Err(_) => match Repository::clone(URL, cano_dir.clone()) {
            Ok(repo) => {
                cloned = true;
                repo
            }
            Err(e) => {
                eprintln!("Failed to clone repository: {e}");
                exit(1);
            }
        },
    };
    println!("Cano source tree present at: {cano_dir}");

    let _ = match File::open(INSTALL_FILE_PATH) {
        Err(_) => {
            build_install(&cano_dir);
        }
        Ok(_) => {
            if cloned == false {
                let remote_branch = "main";
                let mut remote = match repo.find_remote("origin") {
                    Ok(remote) => remote,
                    Err(e) => {
                        eprintln!("Failed to find remote: {e}");
                        exit(1);
                    }
                };

                let mut fo = git2::FetchOptions::new();
                fo.download_tags(git2::AutotagOption::All);
                let _ = remote.fetch(&[remote_branch], Some(&mut fo), None);

                let stats = remote.stats();
                if stats.total_objects() != 0 {
                    let fetch_head = repo.find_reference("FETCH_HEAD").expect("Failure");
                    let _ = match do_merge(
                        &repo,
                        remote_branch,
                        repo.reference_to_annotated_commit(&fetch_head)
                            .expect("Failure"),
                    ) {
                        Ok(_) => (),
                        Err(e) => {
                            eprintln!("Failed to merge {e}");
                            exit(1);
                        }
                    };
                    build_install(&cano_dir);
                } else {
                    println!("Latest version of cano installed.")
                }
            } else {
                build_install(&cano_dir);
            }
        }
    };
}
