use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(AiAnalyses::Table)
                    .add_column(
                        ColumnDef::new(AiAnalyses::Status)
                            .string_len(16)
                            .not_null()
                            .default("success"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(AiAnalyses::Table)
                    .drop_column(AiAnalyses::Status)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum AiAnalyses {
    Table,
    Status,
}
