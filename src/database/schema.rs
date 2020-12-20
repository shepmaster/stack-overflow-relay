table! {
    notifications (id) {
        id -> Int4,
        account_id -> Int4,
        text -> Text,
        created_at -> Timestamptz,
    }
}

table! {
    registrations (account_id) {
        account_id -> Int4,
        access_token -> Text,
    }
}

joinable!(notifications -> registrations (account_id));

allow_tables_to_appear_in_same_query!(notifications, registrations,);
