use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ReviewSessions::Table)
                    .add_column(
                        ColumnDef::new(ReviewSessions::PrimedSessionId)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ReviewSessions::Table)
                    .drop_column(ReviewSessions::PrimedSessionId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ReviewSessions {
    Table,
    PrimedSessionId,
}
