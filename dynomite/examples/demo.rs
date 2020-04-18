use dynomite::{
    dynamodb::{
        AttributeDefinition, CreateTableInput, DynamoDb, DynamoDbClient, GetItemInput,
        KeySchemaElement, ProvisionedThroughput, PutItemInput,
    },
    FromAttributes, Item,
};

#[cfg(feature = "default")]
use rusoto_core_default::Region;

#[cfg(feature = "rustls")]
use rusoto_core_rustls::Region;

use tokio::runtime::Runtime;
use uuid::Uuid;

#[derive(Item, Debug, Clone)]
pub struct Book {
    #[dynomite(partition_key)]
    id: Uuid,
    #[dynomite(rename = "bookTitle")]
    title: String,
}

// this will create a rust book shelf in your aws account!
fn main() {
    env_logger::init();
    let mut rt = Runtime::new().expect("failed to initialize futures runtime");
    // create rusoto client
    let client = DynamoDbClient::new(Region::default());

    // create a book table with a single string (S) primary key.
    // if this table does not already exists
    // this may take a second or two to provision.
    // it will fail if this table already exists but that's okay,
    // this is just an example :)
    let table_name = "books".to_string();
    let _ = rt.block_on(client.create_table(CreateTableInput {
        table_name: table_name.clone(),
        key_schema: vec![KeySchemaElement {
            attribute_name: "id".into(),
            key_type: "HASH".into(),
        }],
        attribute_definitions: vec![AttributeDefinition {
            attribute_name: "id".into(),
            attribute_type: "S".into(),
        }],
        provisioned_throughput: Some(ProvisionedThroughput {
            read_capacity_units: 1,
            write_capacity_units: 1,
        }),
        ..CreateTableInput::default()
    }));

    let book = Book {
        id: Uuid::new_v4(),
        title: "rust".into(),
    };

    // print the key for this book
    // requires bringing `dynomite::Item` into scope
    println!("book.key() {:#?}", book.key());

    // add a book to the shelf
    println!(
        "put_item() result {:#?}",
        rt.block_on(client.put_item(PutItemInput {
            table_name: table_name.clone(),
            item: book.clone().into(), // <= convert book into it's attribute map representation
            ..PutItemInput::default()
        }))
    );

    println!(
        "put_item() result {:#?}",
        rt.block_on(
            client.put_item(PutItemInput {
                table_name: table_name.clone(),
                // convert book into it's attribute map representation
                item: Book {
                    id: Uuid::new_v4(),
                    title: "rust and beyond".into(),
                }
                .into(),
                ..PutItemInput::default()
            })
        )
    );

    // get the "rust' book by the Book type's generated key
    println!(
        "get_item() result {:#?}",
        rt.block_on(client.get_item(GetItemInput {
            table_name: table_name,
            key: book.key(), // get a book by key
            ..GetItemInput::default()
        }))
        .map(|result| result.item.map(Book::from_attrs)) // attempt to convert a attribute map to a book type
    );
}
