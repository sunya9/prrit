use crate::commands::upload::{upload, UploadOptions};
use crate::context::Context;
use crate::refs_for::parse_for_ref;
use crate::remote_helper::{serve, HelperIo};

/// Entry point for `git-remote-prrit <remote> <url>`; git strips the `prrit::`
/// prefix, so <url> is the name of the real remote to push through.
pub fn remote_helper(ctx: &Context<'_>, io: &mut dyn HelperIo, remote: Option<&str>, url: Option<&str>) -> bool {
    let (Some(remote), Some(url)) = (remote, url) else {
        ctx.err("usage: git-remote-prrit <remote> <url>  (invoked by git, not by hand)");
        return false;
    };
    if ctx.git.remote_url(url).is_none() {
        ctx.err(format!("prrit: remote \"{remote}\" points at \"{url}\", which is not a git remote; expected prrit::<remote>"));
        return false;
    }

    serve(io, |src, dst| {
        let for_ref = parse_for_ref(dst)?;
        upload(ctx, &UploadOptions { remote: url, base: &for_ref.base, draft: for_ref.draft, ref_name: src })
    });
    true
}
