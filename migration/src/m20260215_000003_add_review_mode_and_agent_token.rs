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
                        ColumnDef::new(ReviewSessions::ReviewMode)
                            .string()
                            .not_null()
                            .default("PostHoc"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ReviewSessions::Table)
                    .add_column(ColumnDef::new(ReviewSessions::AgentToken).string().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ReviewSessions::Table)
                    .drop_column(ReviewSessions::AgentToken)
                    .drop_column(ReviewSessions::ReviewMode)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ReviewSessions {
    Table,
    ReviewMode,
    AgentToken,
}
