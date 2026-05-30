use kalamdb_dba::{models::NotificationRow, NotificationsRepository, SharedTableRepository};

#[test]
fn shared_repository_api_is_exposed() {
    let _shared_notifications_ctor = SharedTableRepository::<NotificationRow>::new;
    let _notifications_ctor = NotificationsRepository::new;
}
