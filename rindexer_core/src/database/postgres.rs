use bb8::{Pool, RunError};
use bb8_postgres::PostgresConnectionManager;
use std::error::Error;
use std::{env, str};

// External crates
use bytes::BytesMut;
use dotenv::dotenv;
use ethers::abi::Token;
use ethers::types::{Address, Bytes, H128, H160, H256, H512, U128, U256, U512, U64};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use rust_decimal::Decimal;
use thiserror::Error;
use tokio_postgres::types::{to_sql_checked, IsNull, ToSql, Type as PgType};
use tokio_postgres::{Error as PgError, NoTls, Row, Statement, Transaction as PgTransaction};
use tracing::{debug, info};

use crate::generator::{
    extract_event_names_and_signatures_from_abi, generate_abi_name_properties, read_abi_items,
    ABIInput, EventInfo, GenerateAbiPropertiesType,
};
// Internal modules
use crate::generator::build::{contract_name_to_filter_name, is_filter};
use crate::helpers::camel_to_snake;
use crate::manifest::yaml::{Indexer, Manifest, ProjectType};

// pub fn database_user() -> Result<String, env::VarError> {
//     dotenv().ok();
//     env::var("DATABASE_USER")
// }

/// Constructs a PostgresSQL connection string using environment variables.
///
/// This function reads database connection details from environment variables
/// and constructs a connection string in the format required by PostgresSQL.
///
/// # Returns
///
/// A `Result` containing the connection string on success, or an `env::VarError` on failure.
pub fn connection_string() -> Result<String, env::VarError> {
    dotenv().ok();
    let connection = env::var("DATABASE_URL")?;
    Ok(connection)
}
/// Constructs a PostgreSQL connection string from environment variables,
/// encoding the password to be URL-safe.
///
/// # Returns
///
/// Returns a `Result` containing the PostgreSQL connection string if successful,
/// or an `env::VarError` if any of the required environment variables are not set.
// pub fn connection_string_as_url() -> Result<String, env::VarError> {
//     dotenv().ok();
//     let password =
//         utf8_percent_encode(&env::var("DATABASE_PASSWORD")?, NON_ALPHANUMERIC).to_string();
//     Ok(format!(
//         "postgresql://{}:{}@{}:{}/{}",
//         database_user()?,
//         password,
//         env::var("DATABASE_HOST")?,
//         env::var("DATABASE_PORT")?,
//         env::var("DATABASE_NAME")?
//     ))
// }

pub struct PostgresClient {
    pool: Pool<PostgresConnectionManager<NoTls>>,
}

#[derive(Error, Debug)]
pub enum PostgresConnectionError {
    #[error("The database connection string is wrong please check your environment: {0}")]
    DatabaseConnectionConfigWrong(env::VarError),

    #[error("Connection pool error: {0}")]
    ConnectionPoolError(tokio_postgres::Error),
}

#[derive(Error, Debug)]
pub enum PostgresError {
    #[error("PgError {0}")]
    PgError(PgError),

    #[error("Connection pool error: {0}")]
    ConnectionPoolError(RunError<tokio_postgres::Error>),
}

pub struct PostgresTransaction {
    pub transaction: PgTransaction<'static>,
}

impl PostgresClient {
    pub async fn new() -> Result<Self, PostgresConnectionError> {
        let manager = PostgresConnectionManager::new_from_stringlike(
            connection_string().map_err(PostgresConnectionError::DatabaseConnectionConfigWrong)?,
            NoTls,
        )
        .unwrap();
        let pool = Pool::builder()
            .build(manager)
            .await
            .map_err(PostgresConnectionError::ConnectionPoolError)?;
        Ok(Self { pool })
    }

    pub async fn batch_execute(&self, sql: &str) -> Result<(), PostgresError> {
        let conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        conn.batch_execute(sql)
            .await
            .map_err(PostgresError::PgError)
    }

    pub async fn execute<T>(
        &self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, PostgresError>
    where
        T: ?Sized + tokio_postgres::ToStatement,
    {
        let conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        conn.execute(query, params)
            .await
            .map_err(PostgresError::PgError)
    }

    pub async fn prepare(
        &self,
        query: &str,
        parameter_types: &[PgType],
    ) -> Result<Statement, PostgresError> {
        let conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        conn.prepare_typed(query, parameter_types)
            .await
            .map_err(PostgresError::PgError)
    }

    pub async fn transaction(&self) -> Result<PostgresTransaction, PostgresError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        let transaction = conn.transaction().await.map_err(PostgresError::PgError)?;

        // Wrap the transaction in a static lifetime
        let boxed_transaction: Box<PgTransaction<'static>> =
            unsafe { std::mem::transmute(Box::new(transaction)) };
        Ok(PostgresTransaction {
            transaction: *boxed_transaction,
        })
    }

    pub async fn query<T>(
        &self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, PostgresError>
    where
        T: ?Sized + tokio_postgres::ToStatement,
    {
        let conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        let rows = conn
            .query(query, params)
            .await
            .map_err(PostgresError::PgError)?;
        Ok(rows)
    }

    pub async fn query_one<T>(
        &self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, PostgresError>
    where
        T: ?Sized + tokio_postgres::ToStatement,
    {
        let conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        let row = conn
            .query_one(query, params)
            .await
            .map_err(PostgresError::PgError)?;
        Ok(row)
    }

    pub async fn query_one_or_none<T>(
        &self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, PostgresError>
    where
        T: ?Sized + tokio_postgres::ToStatement,
    {
        let conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        let row = conn
            .query_opt(query, params)
            .await
            .map_err(PostgresError::PgError)?;
        Ok(row)
    }

    pub async fn batch_insert<T>(
        &self,
        query: &T,
        params_list: Vec<Vec<Box<dyn ToSql + Send + Sync>>>,
    ) -> Result<(), PostgresError>
    where
        T: ?Sized + tokio_postgres::ToStatement,
    {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(PostgresError::ConnectionPoolError)?;
        let transaction = conn.transaction().await.map_err(PostgresError::PgError)?;

        for params in params_list {
            let params_refs: Vec<&(dyn ToSql + Sync)> = params
                .iter()
                .map(|param| param.as_ref() as &(dyn ToSql + Sync))
                .collect();
            transaction
                .execute(query, &params_refs)
                .await
                .map_err(PostgresError::PgError)?;
        }

        transaction.commit().await.map_err(PostgresError::PgError)?;
        Ok(())
    }
}

pub async fn setup_postgres(manifest: &Manifest) -> Result<PostgresClient, Box<dyn Error>> {
    info!("Setting up postgres");
    let client = PostgresClient::new().await?;

    // No-code will ignore this as it must have tables if postgres used
    if !manifest.storage.postgres_disable_create_tables()
        || manifest.project_type == ProjectType::NoCode
    {
        info!("Creating tables for all indexers");
        for indexer in &manifest.indexers {
            info!("Creating tables for indexer: {}", indexer.name);
            let sql = create_tables_for_indexer_sql(indexer);
            debug!("{}", sql);
            client.batch_execute(&sql).await?;
            info!("Created tables for indexer: {}", indexer.name);
        }
    }

    Ok(client)
}

/// Converts a Solidity ABI type to a corresponding SQL data type.
///
/// This function maps various Solidity types to their appropriate SQL types.
/// If the Solidity type is an array, the corresponding SQL type will also be an array.
///
/// # Arguments
///
/// * `abi_type` - A string slice that holds the Solidity ABI type.
///
/// # Returns
///
/// A `String` representing the corresponding SQL data type.
///
/// # Panics
///
/// The function will panic if it encounters an unsupported Solidity type.
pub fn solidity_type_to_db_type(abi_type: &str) -> String {
    let is_array = abi_type.ends_with("[]");
    let base_type = abi_type.trim_end_matches("[]");

    let sql_type = match base_type {
        "address" => "CHAR(42)",
        "bool" => "BOOLEAN",
        "int256" | "uint256" => "VARCHAR(78)",
        "int64" | "uint64" | "int128" | "uint128" => "NUMERIC",
        "int32" | "uint32" => "INTEGER",
        "string" => "TEXT",
        t if t.starts_with("bytes") => "BYTEA",
        "uint8" | "uint16" | "int8" | "int16" => "SMALLINT",
        _ => panic!("Unsupported type {}", abi_type),
    };

    // Return the SQL type, appending array brackets if necessary
    if is_array {
        format!("{}[]", sql_type)
    } else {
        sql_type.to_string()
    }
}

/// Generates a vector of strings based on ABI input properties and a specified property type.
///
/// # Arguments
///
/// * `inputs` - A slice of `ABIInput` containing the ABI input details.
/// * `property_type` - The type of property generation (e.g., with data types or names only).
///
/// # Returns
///
/// A `Vec<String>` containing the generated properties as strings.
fn generate_columns(inputs: &[ABIInput], property_type: &GenerateAbiPropertiesType) -> Vec<String> {
    generate_abi_name_properties(inputs, property_type, None)
        .iter()
        .map(|m| m.value.clone())
        .collect()
}

/// Generates a vector of columns with data types based on ABI input properties.
///
/// # Arguments
///
/// * `inputs` - A slice of `ABIInput` containing the ABI input details.
///
/// # Returns
///
/// A `Vec<String>` containing the column definitions with data types.
pub fn generate_columns_with_data_types(inputs: &[ABIInput]) -> Vec<String> {
    generate_columns(inputs, &GenerateAbiPropertiesType::PostgresWithDataTypes)
}

/// Generates a vector of column names based on ABI input properties.
///
/// # Arguments
///
/// * `inputs` - A slice of `ABIInput` containing the ABI input details.
///
/// # Returns
///
/// A `Vec<String>` containing the column names.
pub fn generate_columns_names_only(inputs: &[ABIInput]) -> Vec<String> {
    generate_columns(inputs, &GenerateAbiPropertiesType::PostgresColumnsNamesOnly)
}

/// Generates SQL queries to create tables based on provided event information.
///
/// # Arguments
///
/// * `abi_inputs` - A slice of `EventInfo` containing the event details.
/// * `schema_name` - The name of the database schema.
///
/// # Returns
///
/// A `String` containing the SQL queries to create the tables.
fn generate_event_table_sql(abi_inputs: &[EventInfo], schema_name: &str) -> String {
    abi_inputs
        .iter()
        .map(|event_info| {
            let table_name = format!("{}.{}", schema_name, camel_to_snake(&event_info.name));
            info!("Creating table: {}", table_name);
            format!(
                "CREATE TABLE IF NOT EXISTS {} (\
                rindexer_id SERIAL PRIMARY KEY, \
                contract_address CHAR(66), \
                {}, \
                tx_hash CHAR(66), \
                block_number NUMERIC, \
                block_hash CHAR(66)\
            );",
                table_name,
                generate_columns_with_data_types(&event_info.inputs).join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generates SQL queries to create internal event tables and insert initial data.
///
/// This function creates SQL tables to track the last synced block for each event on different networks.
/// It constructs a table for each event and inserts an initial record for each network.
///
/// # Arguments
///
/// * `abi_inputs` - A slice of `EventInfo` containing the event details.
/// * `schema_name` - The name of the schema.
/// * `networks` - A vector of strings representing the network names.
///
/// # Returns
///
/// A `String` containing the SQL queries to create the tables and insert initial data.
fn generate_internal_event_table_sql(
    abi_inputs: &[EventInfo],
    schema_name: &str,
    networks: Vec<String>,
) -> String {
    abi_inputs.iter().map(|event_info| {
        let table_name = format!(
            "rindexer_internal.{}_{}",
            schema_name,
            camel_to_snake(&event_info.name)
        );

        let create_table_query = format!(
            r#"CREATE TABLE IF NOT EXISTS {} ("network" TEXT PRIMARY KEY, "last_synced_block" NUMERIC);"#,
            table_name
        );

        let insert_queries = networks.iter().map(|network| {
            format!(
                r#"INSERT INTO {} ("network", "last_synced_block") VALUES ('{}', 0) ON CONFLICT ("network") DO NOTHING;"#,
                table_name,
                network
            )
        }).collect::<Vec<_>>().join("\n");

        format!("{}\n{}", create_table_query, insert_queries)
    }).collect::<Vec<_>>().join("\n")
}

/// Generates the schema name for the given indexer.
pub fn indexer_contract_schema_name(indexer_name: &str, contract_name: &str) -> String {
    format!(
        "{}_{}",
        camel_to_snake(indexer_name),
        camel_to_snake(contract_name)
    )
}

/// Generates SQL queries to create tables and schemas for the given indexer.
///
/// This function constructs SQL queries to create the necessary schemas and tables based on the provided indexer configuration.
///
/// # Arguments
///
/// * `indexer` - A reference to the `Indexer` containing the configuration details.
///
/// # Returns
///
/// A `String` containing the SQL queries to create the schemas and tables.
pub fn create_tables_for_indexer_sql(indexer: &Indexer) -> String {
    let mut sql = "CREATE SCHEMA IF NOT EXISTS rindexer_internal;".to_string();

    for contract in &indexer.contracts {
        let contract_name = if is_filter(contract) {
            contract_name_to_filter_name(&contract.name)
        } else {
            contract.name.clone()
        };
        if let Ok(abi_items) = read_abi_items(contract) {
            if let Ok(event_names) = extract_event_names_and_signatures_from_abi(&abi_items) {
                let schema_name = indexer_contract_schema_name(&indexer.name, &contract_name);
                sql.push_str(format!("CREATE SCHEMA IF NOT EXISTS {};", schema_name).as_str());
                info!("Creating schema if not exists: {}", schema_name);

                let networks: Vec<String> =
                    contract.details.iter().map(|d| d.network.clone()).collect();
                sql.push_str(&generate_event_table_sql(&event_names, &schema_name));
                sql.push_str(&generate_internal_event_table_sql(
                    &event_names,
                    &schema_name,
                    networks,
                ));
            }
        }
    }

    sql
}

pub fn drop_tables_for_indexer_sql(indexer: &Indexer) -> String {
    let mut sql = "DROP SCHEMA IF EXISTS rindexer_internal CASCADE;".to_string();

    for contract in &indexer.contracts {
        let contract_name = if is_filter(contract) {
            contract_name_to_filter_name(&contract.name)
        } else {
            contract.name.clone()
        };
        let schema_name = indexer_contract_schema_name(&indexer.name, &contract_name);
        sql.push_str(format!("DROP SCHEMA IF EXISTS {} CASCADE;", schema_name).as_str());
    }

    sql
}

/// Generates a SQL VALUES clause with injected parameters.
///
/// This function constructs a VALUES clause for a SQL statement with a specified number of
/// parameters, formatted as `$1, $2, ..., $count`.
///
/// # Arguments
///
/// * `count` - The number of parameters to generate.
///
/// # Returns
///
/// A `String` containing the SQL VALUES clause with the injected parameters.
pub fn generate_injected_param(count: usize) -> String {
    let params = (1..=count)
        .map(|i| format!("${}", i))
        .collect::<Vec<_>>()
        .join(", ");
    format!("VALUES({})", params)
}

pub fn generated_insert_query_for_event(
    event_info: &EventInfo,
    indexer_name: &str,
    contract_name: &str,
) -> String {
    let columns = generate_columns_names_only(&event_info.inputs);
    let schema_name = indexer_contract_schema_name(indexer_name, contract_name);
    format!(
        "INSERT INTO {}.{} (contract_address, {}, \"tx_hash\", \"block_number\", \"block_hash\") {}",
        schema_name,
        camel_to_snake(&event_info.name),
        &columns.join(", "),
        generate_injected_param(4 + columns.len())
    )
}

#[derive(Debug)]
pub enum EthereumSqlTypeWrapper<'a> {
    U64(&'a U64),
    VecU64(&'a Vec<U64>),
    U128(&'a U128),
    VecU128(&'a Vec<U128>),
    U256(&'a U256),
    VecU256(&'a Vec<U256>),
    U512(&'a U512),
    VecU512(Vec<U512>),
    H128(&'a H128),
    VecH128(&'a Vec<H128>),
    H160(&'a H160),
    VecH160(&'a Vec<H160>),
    H256(&'a H256),
    VecH256(&'a Vec<H256>),
    H512(&'a H512),
    VecH512(&'a Vec<H512>),
    Address(&'a Address),
    VecAddress(&'a Vec<Address>),
    Bool(&'a bool),
    VecBool(&'a Vec<bool>),
    U32(&'a u32),
    VecU32(&'a Vec<u32>),
    U16(&'a u16),
    VecU16(&'a Vec<u16>),
    U8(&'a u8),
    VecU8(&'a Vec<u8>),
    String(&'a String),
    VecString(&'a Vec<String>),
    Bytes(&'a Bytes),
    VecBytes(&'a Vec<Bytes>),
}

impl<'a> EthereumSqlTypeWrapper<'a> {
    pub fn raw_name(&self) -> &'static str {
        match self {
            EthereumSqlTypeWrapper::U64(_) => "U64",
            EthereumSqlTypeWrapper::VecU64(_) => "VecU64",
            EthereumSqlTypeWrapper::U128(_) => "U128",
            EthereumSqlTypeWrapper::VecU128(_) => "VecU128",
            EthereumSqlTypeWrapper::U256(_) => "U256",
            EthereumSqlTypeWrapper::VecU256(_) => "VecU256",
            EthereumSqlTypeWrapper::U512(_) => "U512",
            EthereumSqlTypeWrapper::VecU512(_) => "VecU512",
            EthereumSqlTypeWrapper::H128(_) => "H128",
            EthereumSqlTypeWrapper::VecH128(_) => "VecH128",
            EthereumSqlTypeWrapper::H160(_) => "H160",
            EthereumSqlTypeWrapper::VecH160(_) => "VecH160",
            EthereumSqlTypeWrapper::H256(_) => "H256",
            EthereumSqlTypeWrapper::VecH256(_) => "VecH256",
            EthereumSqlTypeWrapper::H512(_) => "H512",
            EthereumSqlTypeWrapper::VecH512(_) => "VecH512",
            EthereumSqlTypeWrapper::Address(_) => "Address",
            EthereumSqlTypeWrapper::VecAddress(_) => "VecAddress",
            EthereumSqlTypeWrapper::Bool(_) => "Bool",
            EthereumSqlTypeWrapper::VecBool(_) => "VecBool",
            EthereumSqlTypeWrapper::U32(_) => "U32",
            EthereumSqlTypeWrapper::VecU32(_) => "VecU32",
            EthereumSqlTypeWrapper::U16(_) => "U16",
            EthereumSqlTypeWrapper::VecU16(_) => "VecU16",
            EthereumSqlTypeWrapper::U8(_) => "U8",
            EthereumSqlTypeWrapper::VecU8(_) => "VecU8",
            EthereumSqlTypeWrapper::String(_) => "String",
            EthereumSqlTypeWrapper::VecString(_) => "VecString",
            EthereumSqlTypeWrapper::Bytes(_) => "Bytes",
            EthereumSqlTypeWrapper::VecBytes(_) => "VecBytes",
        }
    }
}

/// Converts a Solidity ABI type to a corresponding Ethereum SQL type wrapper.
///
/// This function maps various Solidity types to their appropriate Ethereum SQL type wrappers.
///
/// # Arguments
///
/// * `abi_type` - A string slice that holds the Solidity ABI type.
///
/// # Returns
///
/// An `Option<EthereumSqlTypeWrapper>` containing the corresponding Ethereum SQL type wrapper if the type is supported, or `None` if the type is unsupported.
///
pub fn solidity_type_to_ethereum_sql_type_wrapper<'a>(
    abi_type: &str,
) -> Option<EthereumSqlTypeWrapper<'a>> {
    static U64_DEFAULT: U64 = U64::zero();
    static VEC_U64_DEFAULT: Vec<U64> = Vec::new();
    static U128_DEFAULT: U128 = U128::zero();
    static VEC_U128_DEFAULT: Vec<U128> = Vec::new();
    static U256_DEFAULT: U256 = U256::zero();
    static VEC_U256_DEFAULT: Vec<U256> = Vec::new();
    // NOT USED HERE
    // static U512_DEFAULT: U512 = U512::zero();
    // static VEC_U512_DEFAULT: Vec<U512> = Vec::new();
    // static H128_DEFAULT: H128 = H128::zero();
    // static VEC_H128_DEFAULT: Vec<H128> = Vec::new();
    // static H160_DEFAULT: H160 = H160::zero();
    // static VEC_H160_DEFAULT: Vec<H160> = Vec::new();
    // static H256_DEFAULT: H256 = H256::zero();
    // static VEC_H256_DEFAULT: Vec<H256> = Vec::new();
    // static H512_DEFAULT: H512 = H512::zero();
    // static VEC_H512_DEFAULT: Vec<H512> = Vec::new();
    static ADDRESS_DEFAULT: Address = Address::zero();
    static VEC_ADDRESS_DEFAULT: Vec<Address> = Vec::new();
    static BOOL_DEFAULT: bool = false;
    static VEC_BOOL_DEFAULT: Vec<bool> = Vec::new();
    static U32_DEFAULT: u32 = 0;
    static VEC_U32_DEFAULT: Vec<u32> = Vec::new();
    static U16_DEFAULT: u16 = 0;
    static VEC_U16_DEFAULT: Vec<u16> = Vec::new();
    static U8_DEFAULT: u8 = 0;
    static VEC_U8_DEFAULT: Vec<u8> = Vec::new();
    static STRING_DEFAULT: String = String::new();
    static VEC_STRING_DEFAULT: Vec<String> = Vec::new();
    static BYTES_DEFAULT: Bytes = Bytes::new();
    static VEC_BYTES_DEFAULT: Vec<Bytes> = Vec::new();

    match abi_type {
        "string" => Some(EthereumSqlTypeWrapper::String(&STRING_DEFAULT)),
        "string[]" => Some(EthereumSqlTypeWrapper::VecString(&VEC_STRING_DEFAULT)),
        "address" => Some(EthereumSqlTypeWrapper::Address(&ADDRESS_DEFAULT)),
        "address[]" => Some(EthereumSqlTypeWrapper::VecAddress(&VEC_ADDRESS_DEFAULT)),
        "bool" => Some(EthereumSqlTypeWrapper::Bool(&BOOL_DEFAULT)),
        "bool[]" => Some(EthereumSqlTypeWrapper::VecBool(&VEC_BOOL_DEFAULT)),
        "int256" | "uint256" => Some(EthereumSqlTypeWrapper::U256(&U256_DEFAULT)),
        "int256[]" | "uint256[]" => Some(EthereumSqlTypeWrapper::VecU256(&VEC_U256_DEFAULT)),
        "int128" | "uint128" => Some(EthereumSqlTypeWrapper::U128(&U128_DEFAULT)),
        "int128[]" | "uint128[]" => Some(EthereumSqlTypeWrapper::VecU128(&VEC_U128_DEFAULT)),
        "int64" | "uint64" => Some(EthereumSqlTypeWrapper::U64(&U64_DEFAULT)),
        "int64[]" | "uint64[]" => Some(EthereumSqlTypeWrapper::VecU64(&VEC_U64_DEFAULT)),
        "int32" | "uint32" => Some(EthereumSqlTypeWrapper::U32(&U32_DEFAULT)),
        "int32[]" | "uint32[]" => Some(EthereumSqlTypeWrapper::VecU32(&VEC_U32_DEFAULT)),
        "int16" | "uint16" => Some(EthereumSqlTypeWrapper::U16(&U16_DEFAULT)),
        "int16[]" | "uint16[]" => Some(EthereumSqlTypeWrapper::VecU16(&VEC_U16_DEFAULT)),
        "int8" | "uint8" => Some(EthereumSqlTypeWrapper::U8(&U8_DEFAULT)),
        "int8[]" | "uint8[]" => Some(EthereumSqlTypeWrapper::VecU8(&VEC_U8_DEFAULT)),
        t if t.starts_with("bytes") && t.contains("[]") => {
            Some(EthereumSqlTypeWrapper::VecBytes(&VEC_BYTES_DEFAULT))
        }
        t if t.starts_with("bytes") => Some(EthereumSqlTypeWrapper::Bytes(&BYTES_DEFAULT)),
        _ => None,
    }
}

pub fn map_log_token_to_ethereum_wrapper(token: &Token) -> Option<EthereumSqlTypeWrapper> {
    match &token {
        Token::Address(address) => Some(EthereumSqlTypeWrapper::Address(address)),
        Token::Int(uint) | Token::Uint(uint) => Some(EthereumSqlTypeWrapper::U256(uint)),
        Token::Bool(b) => Some(EthereumSqlTypeWrapper::Bool(b)),
        Token::String(s) => Some(EthereumSqlTypeWrapper::String(s)),
        // TODO! HANDLE THE MORE ADVANCED STRUCT SYSTEMS
        // Token::FixedBytes(bytes) | Token::Bytes(bytes) => Some(EthereumSqlTypeWrapper::Bytes(bytes.into())),
        // Token::FixedArray(tokens) | Token::Array(tokens) => {
        //     let mut wrappers = Vec::new();
        //     for token in tokens {
        //         if let Some(wrapper) = map_log_token_to_ethereum_wrapper(token) {
        //             wrappers.push(wrapper);
        //         }
        //     }
        //     Some(EthereumSqlTypeWrapper::VecAddress(wrappers.iter().map(|w| match w {
        //         EthereumSqlTypeWrapper::Address(address) => address,
        //         _ => unreachable!(),
        //     }).collect()))
        // }
        _ => panic!("Unsupported token type"),
    }
}

impl<'a> From<&'a Address> for EthereumSqlTypeWrapper<'a> {
    fn from(address: &'a Address) -> Self {
        EthereumSqlTypeWrapper::Address(address)
    }
}

impl<'a> ToSql for EthereumSqlTypeWrapper<'a> {
    fn to_sql(
        &self,
        _ty: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            EthereumSqlTypeWrapper::U64(value) => {
                let value = value.to_string();
                Decimal::to_sql(&value.parse::<Decimal>().unwrap(), _ty, out)
            }
            EthereumSqlTypeWrapper::VecU64(values) => {
                let results: Vec<Decimal> = values
                    .iter()
                    .map(|s| s.to_string().parse::<Decimal>().unwrap())
                    .collect();
                if results.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    results.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::U128(value) => {
                let value = value.to_string();
                out.extend_from_slice(value.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecU128(values) => {
                let results: Vec<String> = values.iter().map(|s| s.to_string()).collect();
                if results.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    results.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::U256(value) => {
                let value_str = value.to_string();
                out.extend_from_slice(value_str.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecU256(values) => {
                let results: Vec<String> = values.iter().map(|s| s.to_string()).collect();
                if results.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    results.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::U512(value) => {
                let hex = format!("{:?}", value);
                out.extend_from_slice(hex.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecU512(values) => {
                let hexes: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if hexes.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    hexes.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::H128(value) => {
                let hex = format!("{:?}", value);
                out.extend_from_slice(hex.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecH128(values) => {
                let hexes: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if hexes.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    hexes.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::H160(value) => {
                let hex = format!("{:?}", value);
                out.extend_from_slice(hex.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecH160(values) => {
                let hexes: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if hexes.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    hexes.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::H256(value) => {
                let hex = format!("{:?}", value);
                out.extend_from_slice(hex.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecH256(values) => {
                let hexes: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if hexes.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    hexes.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::H512(value) => {
                let hex = format!("{:?}", value);
                out.extend_from_slice(hex.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecH512(values) => {
                let hexes: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if hexes.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    hexes.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::Address(value) => {
                let hex = format!("{:?}", value);
                String::to_sql(&hex, _ty, out)
            }
            EthereumSqlTypeWrapper::VecAddress(values) => {
                let addresses: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if addresses.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    addresses.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::Bool(value) => bool::to_sql(value, _ty, out),
            EthereumSqlTypeWrapper::VecBool(values) => {
                let bools: Vec<i8> = values.iter().map(|&b| if b { 1 } else { 0 }).collect();
                if bools.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    bools.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::U16(value) => {
                let value = value.to_string();
                out.extend_from_slice(value.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecU16(values) => {
                let results: Vec<String> = values.iter().map(|s| s.to_string()).collect();
                if results.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    results.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::String(value) => String::to_sql(value, _ty, out),
            EthereumSqlTypeWrapper::VecString(values) => {
                if values.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    values.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::Bytes(value) => {
                out.extend_from_slice(value);
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecBytes(values) => {
                let hexes: Vec<String> = values.iter().map(|s| format!("{:?}", s)).collect();
                if hexes.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    hexes.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::U32(value) => {
                let value = value.to_string();
                out.extend_from_slice(value.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecU32(values) => {
                let results: Vec<String> = values.iter().map(|s| s.to_string()).collect();
                if results.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    results.to_sql(_ty, out)
                }
            }
            EthereumSqlTypeWrapper::U8(value) => {
                let value = value.to_string();
                out.extend_from_slice(value.as_bytes());
                Ok(IsNull::No)
            }
            EthereumSqlTypeWrapper::VecU8(values) => {
                let results: Vec<String> = values.iter().map(|s| s.to_string()).collect();
                if results.is_empty() {
                    Ok(IsNull::Yes)
                } else {
                    results.to_sql(_ty, out)
                }
            }
        }
    }

    fn accepts(_ty: &PgType) -> bool {
        true // We accept all types
    }

    to_sql_checked!();
}
