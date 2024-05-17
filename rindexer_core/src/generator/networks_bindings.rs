use crate::manifest::yaml::Network;

/// Generates the provider name for a given network.
///
/// # Arguments
///
/// * `network` - A reference to the `Network` configuration.
///
/// # Returns
///
/// A `String` representing the provider name.
fn network_provider_name(network: &Network) -> String {
    network_provider_name_from_name(&network.name)
}

/// Generates the provider name from the network name.
///
/// # Arguments
///
/// * `network_name` - The name of the network.
///
/// # Returns
///
/// A `String` representing the provider name.
fn network_provider_name_from_name(network_name: &str) -> String {
    format!(
        "{network_name}_PROVIDER",
        network_name = network_name.to_uppercase()
    )
}

/// Generates the function name for the network provider.
///
/// # Arguments
///
/// * `network` - A reference to the `Network` configuration.
///
/// # Returns
///
/// A `String` representing the function name for the network provider.
pub fn network_provider_fn_name(network: &Network) -> String {
    format!(
        "get_{fn_name}",
        fn_name = network_provider_name(network).to_lowercase()
    )
}

/// Generates the function name for the network provider from the network name.
///
/// # Arguments
///
/// * `network_name` - The name of the network.
///
/// # Returns
///
/// A `String` representing the function name for the network provider.
pub fn network_provider_fn_name_by_name(network_name: &str) -> String {
    format!(
        "get_{fn_name}",
        fn_name = network_provider_name_from_name(network_name).to_lowercase()
    )
}

/// Generates the lazy provider code for a given network.
///
/// # Arguments
///
/// * `network` - A reference to the `Network` configuration.
///
/// # Returns
///
/// A `Result` containing the generated lazy provider code as a `String`, or an error if something goes wrong.
fn generate_network_lazy_provider_code(
    network: &Network,
) -> Result<String, Box<dyn std::error::Error>> {
    let code = format!(
        r#"
            static ref {network_name}: Arc<Provider<RetryClient<Http>>> = create_retry_client("{network_url}").expect("Error creating provider");
        "#,
        network_name = network_provider_name(network),
        network_url = network.url
    );
    Ok(code)
}

/// Generates the provider function code for a given network.
///
/// # Arguments
///
/// * `network` - A reference to the `Network` configuration.
///
/// # Returns
///
/// A `Result` containing the generated provider function code as a `String`, or an error if something goes wrong.
fn generate_network_provider_code(network: &Network) -> Result<String, Box<dyn std::error::Error>> {
    let code = format!(
        r#"
            pub fn {fn_name}() -> &'static Arc<Provider<RetryClient<Http>>> {{
                &{provider_lazy_name}
            }}
        "#,
        fn_name = network_provider_fn_name(network),
        provider_lazy_name = network_provider_name(network)
    );
    Ok(code)
}

/// Generates the code for all network providers.
///
/// # Arguments
///
/// * `networks` - A reference to a slice of `Network` configurations.
///
/// # Returns
///
/// A `Result` containing the generated network providers code as a `String`, or an error if something goes wrong.
pub fn generate_networks_code(networks: &[Network]) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = r#"
            use ethers::providers::{Provider, Http, RetryClient};
            use rindexer_core::lazy_static;
            use rindexer_core::provider::create_retry_client;
            use std::sync::Arc;

            lazy_static! {
        "#
    .to_string();

    for network in networks {
        output.push_str(&generate_network_lazy_provider_code(network)?);
    }

    output.push('}');

    for network in networks {
        output.push_str(&generate_network_provider_code(network)?);
    }

    Ok(output)
}
