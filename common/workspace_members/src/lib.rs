use cargo_metadata::MetadataCommand;
use proc_macro::TokenStream;
use quote::quote;
use std::error::Error;

fn get_workspace_crates() -> Result<Vec<String>, Box<dyn Error>> {
    let metadata = MetadataCommand::new().no_deps().exec()?;

    Ok(metadata
        .workspace_packages()
        .into_iter()
        .map(|package| package.name.to_string())
        .collect())
}

#[proc_macro]
pub fn workspace_crates(_input: TokenStream) -> TokenStream {
    match get_workspace_crates() {
        Ok(crate_names) => {
            let crate_strs = crate_names.iter().map(std::string::String::as_str);
            quote! {
                &[#(#crate_strs),*]
            }
        }
        Err(e) => {
            let msg = format!("Failed to get workspace crates: {e}");
            quote! {
                compile_error!(#msg);
            }
        }
    }
    .into()
}
