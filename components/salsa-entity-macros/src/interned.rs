use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Ident, ItemImpl, ItemStruct, Path, Token, VisPublic, Visibility};

// #[salsa::interned(Ty0 in Jar0)]
// #[derive(Eq, PartialEq, Hash, Debug, Clone)]
// struct TyData0 {
//    id: u32
// }

pub(crate) fn interned(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let interned_args = syn::parse_macro_input!(args as InternedArgs);
    let data_struct = syn::parse_macro_input!(input as ItemStruct);
    entity_mod(&interned_args, &data_struct).into()
}

pub struct InternedArgs {
    id_ident: Ident,
    _in_token: Token![in],
    jar_path: Path,
}

impl Parse for InternedArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            id_ident: Parse::parse(input)?,
            _in_token: Parse::parse(input)?,
            jar_path: Parse::parse(input)?,
        })
    }
}

fn entity_mod(args: &InternedArgs, data_struct: &ItemStruct) -> proc_macro2::TokenStream {
    let mod_name = syn::Ident::new(
        &format!(
            "__{}",
            heck::SnakeCase::to_snake_case(&*args.id_ident.to_string())
        ),
        args.id_ident.span(),
    );

    let interned_struct: ItemStruct =
        syn::parse2(id_struct(args)).expect("entity_struct parse failed");
    let id_inherent_impl: ItemImpl =
        syn::parse2(id_inherent_impl(args, data_struct)).expect("entity_inherent_impl parse");
    let ingredients_for_impl: ItemImpl =
        syn::parse2(ingredients_for_impl(args, data_struct)).expect("entity_ingredients_for_impl");
    let as_id_impl: ItemImpl = syn::parse2(as_id_impl(args)).expect("as_id_impl");
    let entity_data_inherent_impl: ItemImpl =
        syn::parse2(data_inherent_impl(args, data_struct)).expect("entity_data_inherent_impl");

    quote! {
        #interned_struct
        #id_inherent_impl
        #ingredients_for_impl
        #as_id_impl

        #data_struct
        #entity_data_inherent_impl
    }
}

fn id_struct(args: &InternedArgs) -> proc_macro2::TokenStream {
    let interned_ident = &args.id_ident;
    quote! {
        #[derive(Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Debug)]
        pub struct #interned_ident(salsa::Id);
    }
}

fn id_inherent_impl(args: &InternedArgs, data_struct: &ItemStruct) -> proc_macro2::TokenStream {
    let InternedArgs {
        id_ident, jar_path, ..
    } = args;
    let data_ident = &data_struct.ident;
    quote! {
        impl #id_ident {
            pub fn data<DB: ?Sized>(self, db: &DB) -> & #data_ident
            where
                DB: salsa::storage::HasJar<#jar_path>,
            {
                let (jar, runtime) = salsa::storage::HasJar::jar(db);
                let ingredients = <#jar_path as salsa::storage::HasIngredientsFor< #id_ident >>::ingredient(jar);
                ingredients.data(runtime, self)
            }
        }
    }
}

fn as_id_impl(args: &InternedArgs) -> proc_macro2::TokenStream {
    let id_ident = &args.id_ident;
    quote! {
        impl salsa::AsId for #id_ident {
            fn as_id(self) -> salsa::Id {
                self.0
            }

            fn from_id(id: salsa::Id) -> Self {
                #id_ident(id)
            }
        }

    }
}

fn ingredients_for_impl(args: &InternedArgs, data_struct: &ItemStruct) -> proc_macro2::TokenStream {
    let InternedArgs {
        id_ident, jar_path, ..
    } = args;
    let data_ident = &data_struct.ident;
    quote! {
        impl salsa::storage::IngredientsFor for #id_ident {
            type Jar = #jar_path;
            type Ingredients = salsa::interned::InternedIngredient<#id_ident, #data_ident>;

            fn create_ingredients<DB>(
                ingredients: &mut salsa::routes::Ingredients<DB>,
            ) -> Self::Ingredients
            where
                DB: salsa::storage::HasJars,
                salsa::storage::Storage<DB>: salsa::storage::HasJar<Self::Jar>,
            {
                let index = ingredients.push(
                    |storage| {
                        let (jar, _) = <_ as salsa::storage::HasJar<Self::Jar>>::jar(storage);
                        <Jar0 as salsa::storage::HasIngredientsFor<Self>>::ingredient(jar)
                    },
                );
                salsa::interned::InternedIngredient::new(index)
            }
        }
    }
}

fn data_inherent_impl(args: &InternedArgs, data_struct: &ItemStruct) -> proc_macro2::TokenStream {
    let InternedArgs {
        id_ident, jar_path, ..
    } = args;
    let data_ident = &data_struct.ident;
    quote! {
        impl #data_ident {
            pub fn intern<DB: ?Sized>(self, db: &DB) -> #id_ident
            where
                DB: salsa::storage::HasJar<#jar_path>,
            {
                let (jar, runtime) = salsa::storage::HasJar::jar(db);
                let ingredients = <#jar_path as salsa::storage::HasIngredientsFor<#id_ident>>::ingredient(jar);
                ingredients.intern(runtime, self)
            }
        }
    }
}
