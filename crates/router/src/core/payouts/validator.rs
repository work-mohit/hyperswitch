use common_utils::ext_traits::ByteSliceExt;
use error_stack::{report, ResultExt};
use masking::PeekInterface;
use router_env::{instrument, tracing};

use crate::{
    core::{
        errors::{self, RouterResult},
        payment_methods::cards,
        utils as core_utils,
    },
    db::StorageInterface,
    logger,
    routes::AppState,
    types::{api::payouts, domain, storage},
    utils::{self},
};

#[cfg(feature = "payouts")]
#[instrument(skip(db))]
pub async fn validate_uniqueness_of_payout_id_against_merchant_id(
    db: &dyn StorageInterface,
    payout_id: &str,
    merchant_id: &str,
) -> RouterResult<Option<storage::Payouts>> {
    let payout = db
        .find_payout_by_merchant_id_payout_id(merchant_id, payout_id)
        .await;

    logger::debug!(?payout);
    match payout {
        Err(err) => {
            if err.current_context().is_db_not_found() {
                // Empty vec should be returned by query in case of no results, this check exists just
                // to be on the safer side. Fixed this, now vector is not returned but should check the flow in detail later.
                Ok(None)
            } else {
                Err(err
                    .change_context(errors::ApiErrorResponse::InternalServerError)
                    .attach_printable("Failed while finding payout_attempt, database error"))
            }
        }
        Ok(payout) => {
            if payout.payout_id == payout_id {
                Ok(Some(payout))
            } else {
                Ok(None)
            }
        }
    }
}

/// Validates the request on below checks
/// - merchant_id passed is same as the one in merchant_account table
/// - payout_id is unique against merchant_id
#[cfg(feature = "payouts")]
pub async fn validate_create_request(
    state: &AppState,
    merchant_account: &domain::MerchantAccount,
    key_store: &domain::MerchantKeyStore,
    req: &payouts::PayoutCreateRequest,
) -> RouterResult<(String, Option<payouts::PayoutMethodData>)> {
    let merchant_id = &merchant_account.merchant_id;

    // Merchant ID
    let predicate = req.merchant_id.as_ref().map(|mid| mid != merchant_id);
    utils::when(predicate.unwrap_or(false), || {
        Err(report!(errors::ApiErrorResponse::InvalidDataFormat {
            field_name: "merchant_id".to_string(),
            expected_format: "merchant_id from merchant account".to_string(),
        })
        .attach_printable("invalid merchant_id in request"))
    })?;

    // Payout token
    let customer_id = req.customer_id.to_owned().map_or("".to_string(), |c| c);
    let payout_method_data = match req.payout_token.to_owned() {
        Some(payout_token) => {
            let pm = cards::get_payment_method_from_hs_locker(
                state,
                key_store,
                &customer_id,
                merchant_id,
                &payout_token,
            )
            .await
            .attach_printable("Failed to fetch payout method details from basilisk")
            .change_context(errors::ApiErrorResponse::PayoutNotFound)?;
            let pm_parsed: payouts::PayoutMethodData = pm
                .peek()
                .as_bytes()
                .to_vec()
                .parse_struct("PayoutMethodData")
                .change_context(errors::ApiErrorResponse::InternalServerError)?;
            Some(pm_parsed)
        }
        None => None,
    };

    // Payout ID
    let db: &dyn StorageInterface = &*state.store;
    let payout_id = core_utils::get_or_generate_uuid("payout_id", req.payout_id.as_ref())?;
    match validate_uniqueness_of_payout_id_against_merchant_id(db, &payout_id, merchant_id)
        .await
        .change_context(errors::ApiErrorResponse::DuplicatePayout {
            payout_id: payout_id.to_owned(),
        })
        .attach_printable_lazy(|| {
            format!(
                "Unique violation while checking payout_id: {} against merchant_id: {}",
                payout_id.to_owned(),
                merchant_id
            )
        })? {
        Some(_) => Err(report!(errors::ApiErrorResponse::DuplicatePayout {
            payout_id
        })),
        None => Ok((payout_id, payout_method_data)),
    }
}
