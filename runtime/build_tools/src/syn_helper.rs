use syn::{GenericArgument, Lifetime, Path};

pub fn replace_lifetime_if_found(path: &mut Path, lifetime_to_replace_with: &str) {
    for segment in &mut path.segments {
        if let syn::PathArguments::AngleBracketed(ref mut angle_args) = segment.arguments {
            for arg in angle_args.args.iter_mut() {
                if let GenericArgument::Lifetime(lifetime) = arg {
                    *lifetime = Lifetime::new(lifetime_to_replace_with, lifetime.span());
                    return;
                }
            }
        }
    }
}
