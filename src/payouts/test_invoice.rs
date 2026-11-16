use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
use secp256k1::{
    hashes::{sha256, Hash},
    Secp256k1, SecretKey,
};

pub fn invoice(amount_msat: Option<u64>, seed: u8) -> String {
    let key = SecretKey::from_slice(&[7u8; 32]).unwrap();
    let hash = sha256::Hash::from_slice(&[seed; 32]).unwrap();
    let builder = InvoiceBuilder::new(Currency::Bitcoin)
        .description("test".into())
        .payment_hash(hash)
        .payment_secret(PaymentSecret([2u8; 32]))
        .duration_since_epoch(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap())
        .min_final_cltv_expiry_delta(144);
    let builder = match amount_msat {
        Some(a) => builder.amount_milli_satoshis(a),
        None => builder,
    };
    builder.build_signed(|m| Secp256k1::new().sign_ecdsa_recoverable(m, &key)).unwrap().to_string()
}
