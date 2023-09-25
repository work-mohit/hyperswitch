use common_utils::pii::{Email, IpAddress};
use error_stack::{IntoReport, ResultExt};
use masking::Secret;
use serde::{Deserialize, Serialize};

use crate::{
    connector::utils::{
        self, to_connector_meta, AddressDetailsData, BrowserInformationData, CardData,
        PaymentsAuthorizeRequestData, PaymentsCaptureRequestData, RefundsRequestData, RouterData,
    },
    core::errors,
    types::{self, api, storage::enums},
};

#[derive(Debug, Serialize)]
pub struct HelcimRouterData<T> {
    pub amount: f64,
    pub router_data: T,
}

impl<T>
    TryFrom<(
        &types::api::CurrencyUnit,
        types::storage::enums::Currency,
        i64,
        T,
    )> for HelcimRouterData<T>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        (currency_unit, currency, amount, item): (
            &types::api::CurrencyUnit,
            types::storage::enums::Currency,
            i64,
            T,
        ),
    ) -> Result<Self, Self::Error> {
        let amount = utils::get_amount_as_f64(currency_unit, amount, currency)?;
        Ok(Self {
            amount,
            router_data: item,
        })
    }
}

//TODO: Fill the struct with respective fields
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelcimPaymentsRequest {
    amount: f64,
    currency: enums::Currency,
    ip_address: Secret<String, IpAddress>,
    card_data: HelcimCard,
    billing_address: HelcimBillingAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    ecommerce: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelcimBillingAddress {
    name: Secret<String>,
    street1: Secret<String>,
    postal_code: Secret<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    street2: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<Email>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelcimCard {
    card_number: cards::CardNumber,
    card_expiry: Secret<String>,
    card_c_v_v: Secret<String>,
}

impl TryFrom<&HelcimRouterData<&types::PaymentsAuthorizeRouterData>> for HelcimPaymentsRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &HelcimRouterData<&types::PaymentsAuthorizeRouterData>,
    ) -> Result<Self, Self::Error> {
        match item.router_data.request.payment_method_data.clone() {
            api::PaymentMethodData::Card(req_card) => {
                let card_data = HelcimCard {
                    card_expiry: req_card
                        .get_card_expiry_month_year_2_digit_with_delimiter("".to_string()),
                    card_number: req_card.card_number,
                    card_c_v_v: req_card.card_cvc,
                };
                let req_address = item
                    .router_data
                    .get_billing()?
                    .to_owned()
                    .address
                    .ok_or_else(utils::missing_field_err("billing.address"))?;

                let billing_address = HelcimBillingAddress {
                    name: req_address.get_full_name()?,
                    street1: req_address.get_line1()?.to_owned(),
                    postal_code: req_address.get_zip()?.to_owned(),
                    street2: req_address.line2,
                    city: req_address.city,
                    email: item.router_data.request.email.clone(),
                };

                let ip_address = item
                    .router_data
                    .request
                    .get_browser_info()?
                    .get_ip_address()?;
                Ok(Self {
                    amount: item.amount.to_owned(),
                    currency: item.router_data.request.currency,
                    ip_address,
                    card_data,
                    billing_address,
                    ecommerce: None,
                })
            }
            _ => Err(errors::ConnectorError::NotImplemented("Payment methods".to_string()).into()),
        }
    }
}

//TODO: Fill the struct with respective fields
// Auth Struct
pub struct HelcimAuthType {
    pub(super) api_key: Secret<String>,
}

impl TryFrom<&types::ConnectorAuthType> for HelcimAuthType {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(auth_type: &types::ConnectorAuthType) -> Result<Self, Self::Error> {
        match auth_type {
            types::ConnectorAuthType::HeaderKey { api_key } => Ok(Self {
                api_key: api_key.to_owned(),
            }),
            _ => Err(errors::ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}
// PaymentsResponse
//TODO: Append the remaining status flags
#[derive(Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HelcimPaymentStatus {
    Approved,
    Declined,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HelcimTransactionType {
    Purchase,
    PreAuth,
    Capture,
    Verify,
}

impl From<HelcimPaymentsResponse> for enums::AttemptStatus {
    fn from(item: HelcimPaymentsResponse) -> Self {
        match item.transaction_type {
            HelcimTransactionType::Purchase => match item.status {
                HelcimPaymentStatus::Approved => Self::Charged,
                HelcimPaymentStatus::Declined => Self::Failure,
            },
            HelcimTransactionType::PreAuth => match item.status {
                HelcimPaymentStatus::Approved => Self::Authorized,
                HelcimPaymentStatus::Declined => Self::AuthorizationFailed,
            },
            HelcimTransactionType::Capture => match item.status {
                HelcimPaymentStatus::Approved => Self::Charged, //Is this the correct status PartialCharged
                HelcimPaymentStatus::Declined => Self::CaptureFailed,
            },
            HelcimTransactionType::Verify => match item.status {
                HelcimPaymentStatus::Approved => Self::AuthenticationSuccessful,
                HelcimPaymentStatus::Declined => Self::AuthenticationFailed,
            },
        }
    }
}

//TODO: Fill the struct with respective fields
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelcimPaymentsResponse {
    status: HelcimPaymentStatus,
    transaction_id: u64,
    #[serde(rename = "type")]
    transaction_type: HelcimTransactionType,
}

impl<F>
    TryFrom<
        types::ResponseRouterData<
            F,
            HelcimPaymentsResponse,
            types::PaymentsAuthorizeData,
            types::PaymentsResponseData,
        >,
    > for types::RouterData<F, types::PaymentsAuthorizeData, types::PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::ResponseRouterData<
            F,
            HelcimPaymentsResponse,
            types::PaymentsAuthorizeData,
            types::PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(types::PaymentsResponseData::TransactionResponse {
                resource_id: types::ResponseId::ConnectorTransactionId(
                    item.response.transaction_id.to_string(),
                ),
                redirection_data: None,
                mandate_reference: None,
                connector_metadata: None,
                network_txn_id: None,
                connector_response_reference_id: None,
            }),
            status: enums::AttemptStatus::from(item.response),
            ..item.data
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HelcimMetaData {
    pub capture_id: u64,
}

// impl utils::MultipleCaptureSyncResponse for HelcimPaymentsResponse {
//     fn get_connector_capture_id(&self) -> String {
//         self.transaction_id.to_string()
//     }

//     fn get_capture_attempt_status(&self) -> diesel_models::enums::AttemptStatus {
//         enums::AttemptStatus::from(self.to_owned())
//     }

//     fn is_capture_response(&self) -> bool {
//         true
//     }

//     fn get_amount_captured(&self) -> Option<i64> {
//         Some(self.amount)
//     }
//     fn get_connector_reference_id(&self) -> Option<String> {
//         None
//     }
// }

impl<F>
    TryFrom<
        types::ResponseRouterData<
            F,
            HelcimPaymentsResponse,
            types::PaymentsSyncData,
            types::PaymentsResponseData,
        >,
    > for types::RouterData<F, types::PaymentsSyncData, types::PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::ResponseRouterData<
            F,
            HelcimPaymentsResponse,
            types::PaymentsSyncData,
            types::PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        match item.data.request.sync_type {
            types::SyncRequestType::SinglePaymentSync => Ok(Self {
                response: Ok(types::PaymentsResponseData::TransactionResponse {
                    resource_id: types::ResponseId::ConnectorTransactionId(
                        item.response.transaction_id.to_string(),
                    ),
                    redirection_data: None,
                    mandate_reference: None,
                    connector_metadata: None,
                    network_txn_id: None,
                    connector_response_reference_id: None,
                }),
                status: enums::AttemptStatus::from(item.response),
                ..item.data
            }),
            types::SyncRequestType::MultipleCaptureSync(_) => {
                Err(errors::ConnectorError::NotImplemented(
                    "manual multiple capture sync".to_string(),
                )
                .into())
                // let capture_sync_response_list =
                //     utils::construct_captures_response_hashmap(vec![item.response]);
                // Ok(Self {
                //     response: Ok(types::PaymentsResponseData::MultipleCaptureResponse {
                //         capture_sync_response_list,
                //     }),
                //     ..item.data
                // })
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelcimCaptureRequest {
    pre_auth_transaction_id: u64,
    amount: f64,
    ip_address: Secret<String, IpAddress>,
    ecommerce: Option<bool>,
}

impl TryFrom<&HelcimRouterData<&types::PaymentsCaptureRouterData>> for HelcimCaptureRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &HelcimRouterData<&types::PaymentsCaptureRouterData>,
    ) -> Result<Self, Self::Error> {
        let ip_address = item
            .router_data
            .request
            .get_browser_info()?
            .get_ip_address()?;
        Ok(Self {
            pre_auth_transaction_id: item
                .router_data
                .request
                .connector_transaction_id
                .parse::<u64>()
                .into_report()
                .change_context(errors::ConnectorError::RequestEncodingFailed)?,
            amount: item.amount,
            ip_address,
            ecommerce: None,
        })
    }
}

impl<F>
    TryFrom<
        types::ResponseRouterData<
            F,
            HelcimPaymentsResponse,
            types::PaymentsCaptureData,
            types::PaymentsResponseData,
        >,
    > for types::RouterData<F, types::PaymentsCaptureData, types::PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::ResponseRouterData<
            F,
            HelcimPaymentsResponse,
            types::PaymentsCaptureData,
            types::PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        let connector_metadata = Some(serde_json::json!(HelcimMetaData {
            capture_id: item.response.transaction_id,
        }));
        Ok(Self {
            response: Ok(types::PaymentsResponseData::TransactionResponse {
                resource_id: types::ResponseId::ConnectorTransactionId(
                    item.response.transaction_id.to_string(),
                ),
                redirection_data: None,
                mandate_reference: None,
                connector_metadata,
                network_txn_id: None,
                connector_response_reference_id: None,
            }),
            status: enums::AttemptStatus::from(item.response),
            ..item.data
        })
    }
}

//TODO: Fill the struct with respective fields
// REFUND :
// Type definition for RefundRequest
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelcimRefundRequest {
    amount: f64,
    original_transaction_id: u64,
    ip_address: Secret<String, IpAddress>,
    ecommerce: Option<bool>,
}

impl<F> TryFrom<&HelcimRouterData<&types::RefundsRouterData<F>>> for HelcimRefundRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &HelcimRouterData<&types::RefundsRouterData<F>>,
    ) -> Result<Self, Self::Error> {
        let helcim_meta_data: HelcimMetaData =
            to_connector_meta(item.router_data.request.connector_metadata.clone())?;
        let original_transaction_id = helcim_meta_data.capture_id;
        let ip_address = item
            .router_data
            .request
            .get_browser_info()?
            .get_ip_address()?;
        Ok(Self {
            amount: item.amount,
            original_transaction_id,
            ip_address,
            ecommerce: None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HelcimRefundTransactionType {
    Refund,
}

//TODO: Fill the struct with respective fields
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefundResponse {
    status: HelcimPaymentStatus,
    transaction_id: u64,
    #[serde(rename = "type")]
    transaction_type: HelcimRefundTransactionType,
}

impl From<RefundResponse> for enums::RefundStatus {
    fn from(item: RefundResponse) -> Self {
        match item.transaction_type {
            HelcimRefundTransactionType::Refund => match item.status {
                HelcimPaymentStatus::Approved => Self::Success,
                HelcimPaymentStatus::Declined => Self::Failure,
            },
        }
    }
}

impl TryFrom<types::RefundsResponseRouterData<api::Execute, RefundResponse>>
    for types::RefundsRouterData<api::Execute>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::RefundsResponseRouterData<api::Execute, RefundResponse>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(types::RefundsResponseData {
                connector_refund_id: item.response.transaction_id.to_string(),
                refund_status: enums::RefundStatus::from(item.response),
            }),
            ..item.data
        })
    }
}

impl TryFrom<types::RefundsResponseRouterData<api::RSync, RefundResponse>>
    for types::RefundsRouterData<api::RSync>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: types::RefundsResponseRouterData<api::RSync, RefundResponse>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(types::RefundsResponseData {
                connector_refund_id: item.response.transaction_id.to_string(),
                refund_status: enums::RefundStatus::from(item.response),
            }),
            ..item.data
        })
    }
}

//TODO: Fill the struct with respective fields
#[derive(Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct HelcimErrorResponse {
    pub status_code: u16,
    pub code: String,
    pub message: String,
    pub reason: Option<String>,
}
