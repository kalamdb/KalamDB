use std::time::Duration;

use kalam_client::SubscriptionManager;

const SUBSCRIPTION_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) async fn close_for_cli_exit(subscription: &mut SubscriptionManager, color: bool) {
    print_unsubscribing(color);
    close_or_warn(subscription).await;
    print_unsubscribed(color);
}

pub(super) async fn close_or_warn(subscription: &mut SubscriptionManager) {
    match tokio::time::timeout(SUBSCRIPTION_CLOSE_TIMEOUT, subscription.close()).await {
        Ok(Ok(())) => {},
        Ok(Err(e)) => eprintln!("Warning: Failed to close subscription cleanly: {}", e),
        Err(_) => eprintln!("Warning: Timed out while closing subscription; exiting anyway"),
    }
}

fn print_unsubscribing(color: bool) {
    if color {
        println!("\n\x1b[33m⚠ Unsubscribing...\x1b[0m");
    } else {
        println!("\n⚠ Unsubscribing...");
    }
}

fn print_unsubscribed(color: bool) {
    if color {
        println!("\x1b[32m✓ Unsubscribed\x1b[0m Back to CLI prompt");
    } else {
        println!("✓ Unsubscribed - Back to CLI prompt");
    }
}
