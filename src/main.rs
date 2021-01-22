//! A Kodi repository server, with specific support for serving addons straight out of Git
//! repositories. Uses an extra directory on disk to cache `.zip` files.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::Path;
use std::fs;

const IDS_XPATH: &str = "/addons/addon/@id";

/// Retrieve addon IDs from repository addon listing file.
// TODO: examine how much work it'd be to
// support non UTF-8 manifests.
fn get_ids(listing: &Path) -> Vec<String> {
    let package = sxd_document::parser::parse(
        &fs::read_to_string(listing).expect("couldn't read listing file"))
        .expect("listing file was invalid XML");
    let document = package.as_document();
    let value = sxd_xpath::evaluate_xpath(&document, IDS_XPATH).expect("failed XPath evaluation");
    println!("IDs: {:?}", value);
    use sxd_xpath::Value;
    use sxd_xpath::nodeset::Node;
    match value {
        Value::Nodeset(set) => {
            set.iter().map(|node| {
                match node {
                    Node::Attribute(attr) => attr.value().to_owned(),
                    node => panic!("invalid node type from xpath evaluation: {:?}", node),
                }
            }).collect()
        }
        val => panic!("invalid value type from xpath evaluation: {:?}", val),
    }
}

#[cfg(feature = "server")]
#[tokio::main]
async fn serve(addons_dir: &Path, listing: &Path, cache_dir: &Path) {
    use warp::Filter;
    use warp::Reply;
    use std::net::SocketAddr;
    use std::sync::Arc;
    // We avoid hitting the filesystem on invalid requests for a number of reasons.
    let ids = Arc::new(get_ids(listing));
    let socket_addr: SocketAddr = "127.0.0.1:9001".parse().unwrap();
    // Note, type errors here are much more helpful
    // when the closure argument type is annotated.
    let filter = warp::path!("addons" / String).and_then(move |id: String| {
        // Workaround what may be a borrow checker limitation.
        // I do happen to know that `Server::run` never returns,
        // so the initial `ids` binding never goes out of scope,
        // which means that reference counted (must less atomically reference counted)
        // shared ownership is unnecessary here.
        // Given that, I could use raw pointers to force the issue.
        // But, I really don't feel like worrying about more `unsafe` than I have to.
        let ids = ids.clone();
        async move {
            if ids.contains(&id) {
                Ok(Cow::Owned(format!("{} exists!", id)))
            } else {
                Err(warp::reject::not_found())
            }
        }
    }).and(warp::header::optional("compression")).map(|processed, compression: Option<bool>| {
        (processed, compression.unwrap_or(false))
    }).and_then(|(processed, compression)| async move {
        if compression {
            Ok(processed)
        } else {
            Err(warp::reject::custom(server::NoGzipW::new(processed)))
        }
    }).with(warp::compression::gzip()).recover(server::handle_no_gzip::<Cow<'_, str>>);
    warp::serve(filter).run(socket_addr).await;
}

#[cfg(feature = "server")]
mod server {
    use warp::Filter;
    use warp::reject::{Reject, Rejection};
    use warp::Reply;
    use core::mem::ManuallyDrop;
    #[derive(Debug)]
    pub(crate) struct NoGzipW<T> {
        // TODO: consider using UnsafeCell?
        // I don't need to mutate while it's held- only to steal it right before
        // the thing holding it drops.
        // I only get an immutable reference to `NoGzipW`,
        // hence my not using `Option` with `.take()` or something.
        never_drop: ManuallyDrop<T>
    }
    impl<T> NoGzipW<T> {
        pub(crate) fn new(val: T) -> Self {
            Self {
                never_drop: ManuallyDrop::new(val),
            }
        }
    }
    impl<T: core::fmt::Debug + Send + Sync + 'static> Reject for NoGzipW<T> {}
    pub(crate) async fn handle_no_gzip<T: Reply + 'static>(reject: Rejection) -> Result<T, Rejection> {
            match reject.find::<NoGzipW<T>>() {
                Some(x) => {
                    // Important Note: If this breaks, I definitely have zero right to complain.
                    // I should look into getting what I need for this to be stably sound
                    // into the `warp` crate.
                    // That said, global reasoning of this crate will not stop being correct.
                    // It is only that `warp` may change to make this *require* that global
                    // reasoning to be done.
                    // Safety: This relies somewhat on an implementation detail of `warp`.
                    // That is, we assume that holding a `Rejection` means we hold a unique
                    // owning pointer to the underlying cause.
                    // This is true at the time of writing (warp 0.3.0), as `Rejection` stores
                    // custom causes in a `Box<dyn Cause>`, with no shared ownership in sight.
                    // Therefore, if we ensure `reject` is not used after this, and that
                    // the stored duplicate inside of `reject` does not run any existing `Drop`
                    // implementation, no logical invariants will be broken.
                    // We ensure that no `Drop` implementation is run via the use of `ManuallyDrop`
                    // inside of `NoGzipW`.
                    Ok(unsafe { ::std::ptr::read(&*x.never_drop) })
                },
                None => Err(reject)
            }
        }
}

#[cfg(not(feature = "server"))]
fn serve(_addons_dir: &Path, _listing: &Path, _cache_dir: &Path) {
    panic!("this binary does not include server functionality")
}

fn write_listing(addons_dir: &Path, output: &Path) {
    todo!("generating a listing")
}

fn main() {
    // TODO: support non UTF-8 paths
    let args: Vec<String> = std::env::args().collect();
    let args: Vec<&str> = args.iter().map(|x| &**x).collect();
    match args[1..] {
        // Generate XML listing of addons given addons directory.
        ["generate", addons_dir, output] => write_listing(Path::new(addons_dir), Path::new(output)),
        ["server", addons_dir, listing] => serve(Path::new(addons_dir),
                                                 Path::new(listing),
                                                 &Path::new(addons_dir).join(".zips")),
        ["server", addons_dir, listing, cache_dir] => serve(Path::new(addons_dir),
                                                            Path::new(listing),
                                                            Path::new(cache_dir)),
        _ => panic!("wrong args"),
    }
}
