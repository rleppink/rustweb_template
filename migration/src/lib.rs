// `sea-orm-cli`-generated migrations elide lifetimes in `&SchemaManager`.
#![allow(elided_lifetimes_in_paths)]

pub use sea_orm_migration::prelude::*;

mod m20240101_000001_create_users;
mod m20240101_000002_create_posts;
mod m20240101_000003_add_password_hash;
mod m20240101_000004_create_password_reset_tokens;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240101_000001_create_users::Migration),
            Box::new(m20240101_000002_create_posts::Migration),
            Box::new(m20240101_000003_add_password_hash::Migration),
            Box::new(m20240101_000004_create_password_reset_tokens::Migration),
        ]
    }
}
