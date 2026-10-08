//! Implementation of [EDHOC] (Ephemeral Diffie-Hellman Over COSE, RFC9528), a lightweight authenticated key
//! exchange for the Internet of Things.
//!
//! The crate provides a high-level interface through the [EdhocInitiator] and the [EdhocResponder]
//! structs. Both these wrap the lower level [State] struct that is mainly used through internal
//! functions in the `edhoc` module. This separation is relevant because the lower level tools are
//! subject of ongoing formal verification, whereas the high-level interfaces aim for good
//! usability.
//!
//! Both [EdhocInitiator] and [EdhocResponder] are used in a type stated way. Following the EDHOC
//! protocol, they generate (or process) messages, progressively provide more information about
//! their peer, and on eventually devolve into an [EdhocInitiatorDone] and [EdhocResponderDone],
//! respectively, through which the EDHOC key material can be obtained.
//!
//! [EDHOC]: https://datatracker.ietf.org/doc/html/rfc9528

pub use super::shared::{Crypto as CryptoTrait, *};
use defmt_or_log::trace;

pub use super::edhoc::*;

/// Starting point for performing EDHOC in the role of the Initiator.
#[derive(Debug)]
pub struct EdhocInitiator<Crypto: CryptoTrait> {
    state: InitiatorStart,       // opaque state
    i: Option<BytesP256ElemLen>, // static public key of myself
    cred_i: Option<Credential>,
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocInitiatorWaitM2<Crypto: CryptoTrait> {
    state: WaitM2, // opaque state
    i: Option<BytesP256ElemLen>,
    cred_i: Option<Credential>,
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocInitiatorProcessingM2<Crypto: CryptoTrait> {
    state: ProcessingM2, // opaque state
    i: Option<BytesP256ElemLen>,
    cred_i: Option<Credential>,
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocInitiatorProcessedM2<Crypto: CryptoTrait> {
    state: ProcessedM2, // opaque state
    cred_i: Option<Credential>,
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocInitiatorWaitM4<Crypto: CryptoTrait> {
    state: WaitM4, // opaque state
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocInitiatorDone<Crypto: CryptoTrait> {
    state: Completed,
    crypto: Crypto,
}

/// Starting point for performing EDHOC in the role of the Responder.
#[derive(Debug)]
pub struct EdhocResponder<Crypto: CryptoTrait> {
    state: ResponderStart, // opaque state
    r: BytesP256ElemLen,   // private authentication key of R
    cred_r: Credential,    // R's full credential
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocResponderProcessedM1<Crypto: CryptoTrait> {
    state: ProcessingM1, // opaque state
    r: BytesP256ElemLen, // private authentication key of R
    cred_r: Credential,  // R's full credential
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocResponderWaitM3<Crypto: CryptoTrait> {
    state: WaitM3, // opaque state
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocResponderProcessingM3<Crypto: CryptoTrait> {
    state: ProcessingM3, // opaque state
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocResponderProcessedM3<Crypto: CryptoTrait> {
    state: ProcessedM3, // opaque state
    crypto: Crypto,
}

#[derive(Debug)]
pub struct EdhocResponderDone<Crypto: CryptoTrait> {
    state: Completed,
    crypto: Crypto,
}

impl<Crypto: CryptoTrait> EdhocResponder<Crypto> {
    pub fn new(
        mut crypto: Crypto,
        method: EDHOCMethod,
        r: BytesP256ElemLen,
        cred_r: Credential,
    ) -> Self {
        trace!("Initializing EdhocResponder");
        let (y, g_y) = crypto.p256_generate_key_pair();

        EdhocResponder {
            state: ResponderStart {
                y,
                g_y,
                method: method.into(),
            },
            r,
            cred_r,
            crypto,
        }
    }

    pub fn process_message_1(
        mut self,
        message_1: &BufferMessage1,
    ) -> Result<(EdhocResponderProcessedM1<Crypto>, ConnId, Option<EADItem>), EDHOCError> {
        trace!("Enter process_message_1");
        let (state, c_i, ead_1) = r_process_message_1(&self.state, &mut self.crypto, message_1)?;

        Ok((
            EdhocResponderProcessedM1 {
                state,
                r: self.r,
                cred_r: self.cred_r,
                crypto: self.crypto,
            },
            c_i,
            ead_1,
        ))
    }
}

impl<Crypto: CryptoTrait> EdhocResponderProcessedM1<Crypto> {
    pub fn prepare_message_2(
        mut self,
        cred_transfer: CredentialTransfer,
        c_r: Option<ConnId>,
        ead_2: &Option<EADItem>,
    ) -> Result<(EdhocResponderWaitM3<Crypto>, BufferMessage2), EDHOCError> {
        trace!("Enter prepare_message_2");
        let c_r = match c_r {
            Some(c_r) => c_r,
            None => generate_connection_identifier_cbor(&mut self.crypto),
        };

        match r_prepare_message_2(
            &self.state,
            &mut self.crypto,
            self.cred_r,
            &self.r,
            c_r,
            cred_transfer,
            ead_2,
        ) {
            Ok((state, message_2)) => Ok((
                EdhocResponderWaitM3 {
                    state,
                    crypto: self.crypto,
                },
                message_2,
            )),
            Err(error) => Err(error),
        }
    }
}

impl<'a, Crypto: CryptoTrait> EdhocResponderWaitM3<Crypto> {
    pub fn parse_message_3(
        mut self,
        message_3: &'a BufferMessage3,
    ) -> Result<(EdhocResponderProcessingM3<Crypto>, IdCred, Option<EADItem>), EDHOCError> {
        trace!("Enter parse_message_3");
        match r_parse_message_3(&mut self.state, &mut self.crypto, message_3) {
            Ok((state, id_cred_i, ead_3)) => Ok((
                EdhocResponderProcessingM3 {
                    state,
                    crypto: self.crypto,
                },
                id_cred_i,
                ead_3,
            )),
            Err(error) => Err(error),
        }
    }
}

impl<'a, Crypto: CryptoTrait> EdhocResponderProcessingM3<Crypto> {
    pub fn verify_message_3(
        mut self,
        cred_i: Credential,
    ) -> Result<(EdhocResponderProcessedM3<Crypto>, [u8; SHA256_DIGEST_LEN]), EDHOCError> {
        trace!("Enter verify_message_3");
        match r_verify_message_3(&mut self.state, &mut self.crypto, cred_i) {
            Ok((state, prk_out)) => Ok((
                EdhocResponderProcessedM3 {
                    state,
                    crypto: self.crypto,
                },
                prk_out,
            )),
            Err(error) => Err(error),
        }
    }
}

impl<Crypto: CryptoTrait> EdhocResponderProcessedM3<Crypto> {
    pub fn prepare_message_4(
        mut self,
        ead_4: &Option<EADItem>,
    ) -> Result<(EdhocResponderDone<Crypto>, BufferMessage4), EDHOCError> {
        trace!("Enter prepare_message_4");
        match r_prepare_message_4(&self.state, &mut self.crypto, ead_4) {
            Ok((state, message_4)) => Ok((
                EdhocResponderDone {
                    state,
                    crypto: self.crypto,
                },
                message_4,
            )),
            Err(error) => Err(error),
        }
    }

    pub fn completed_without_message_4(self) -> Result<EdhocResponderDone<Crypto>, EDHOCError> {
        trace!("Enter completed");
        match r_complete_without_message_4(&self.state) {
            Ok(state) => Ok(EdhocResponderDone {
                state,
                crypto: self.crypto,
            }),
            Err(error) => Err(error),
        }
    }
}

impl<Crypto: CryptoTrait> EdhocResponderDone<Crypto> {
    pub fn edhoc_exporter(
        &mut self,
        label: u8,
        context: &[u8],
        length: usize,
    ) -> [u8; MAX_BUFFER_LEN] {
        let mut context_buf: BytesMaxContextBuffer = [0x00u8; MAX_KDF_CONTEXT_LEN];
        context_buf[..context.len()].copy_from_slice(context);

        edhoc_exporter(
            &self.state,
            &mut self.crypto,
            label,
            &context_buf,
            context.len(),
            length,
        )
    }

    pub fn edhoc_key_update(&mut self, context: &[u8]) -> [u8; SHA256_DIGEST_LEN] {
        let mut context_buf = [0x00u8; MAX_KDF_CONTEXT_LEN];
        context_buf[..context.len()].copy_from_slice(context);

        edhoc_key_update(
            &mut self.state,
            &mut self.crypto,
            &context_buf,
            context.len(),
        )
    }
}

impl<'a, Crypto: CryptoTrait> EdhocInitiator<Crypto> {
    pub fn new(mut crypto: Crypto, method: EDHOCMethod, selected_suite: EDHOCSuite) -> Self {
        trace!("Initializing EdhocInitiator");
        let suites_i = prepare_suites_i(&crypto.supported_suites(), selected_suite.into()).unwrap();
        let (x, g_x) = crypto.p256_generate_key_pair();

        EdhocInitiator {
            state: InitiatorStart {
                x,
                g_x,
                method: method.into(),
                suites_i,
            },
            i: None,
            cred_i: None,
            crypto,
        }
    }

    pub fn set_identity(&mut self, i: BytesP256ElemLen, cred_i: Credential) {
        self.i = Some(i);
        self.cred_i = Some(cred_i);
    }

    pub fn prepare_message_1(
        mut self,
        c_i: Option<ConnId>,
        ead_1: &Option<EADItem>,
    ) -> Result<(EdhocInitiatorWaitM2<Crypto>, EdhocMessageBuffer), EDHOCError> {
        trace!("Enter prepare_message_1");
        let c_i = match c_i {
            Some(c_i) => c_i,
            None => generate_connection_identifier_cbor(&mut self.crypto),
        };

        match i_prepare_message_1(&self.state, &mut self.crypto, c_i, ead_1) {
            Ok((state, message_1)) => Ok((
                EdhocInitiatorWaitM2 {
                    state,
                    i: self.i,
                    cred_i: self.cred_i,
                    crypto: self.crypto,
                },
                message_1,
            )),
            Err(error) => Err(error),
        }
    }

    pub fn compute_ephemeral_secret(&mut self, g_a: &BytesP256ElemLen) -> BytesP256ElemLen {
        self.crypto.p256_ecdh(&self.state.x, g_a)
    }

    pub fn selected_cipher_suite(&self) -> u8 {
        self.state.suites_i[self.state.suites_i.len - 1]
    }
}

impl<'a, Crypto: CryptoTrait> EdhocInitiatorWaitM2<Crypto> {
    pub fn parse_message_2(
        mut self,
        message_2: &'a BufferMessage2,
    ) -> Result<
        (
            EdhocInitiatorProcessingM2<Crypto>,
            ConnId,
            IdCred,
            Option<EADItem>,
        ),
        EDHOCError,
    > {
        trace!("Enter parse_message_2");
        match i_parse_message_2(&self.state, &mut self.crypto, message_2) {
            Ok((state, c_r, id_cred_r, ead_2)) => Ok((
                EdhocInitiatorProcessingM2 {
                    state,
                    i: self.i,
                    cred_i: self.cred_i,
                    crypto: self.crypto,
                },
                c_r,
                id_cred_r,
                ead_2,
            )),
            Err(error) => Err(error),
        }
    }
}

impl<'a, Crypto: CryptoTrait> EdhocInitiatorProcessingM2<Crypto> {
    pub fn set_identity(
        &mut self,
        i: BytesP256ElemLen,
        cred_i: Credential,
    ) -> Result<(), EDHOCError> {
        if self.i.is_some() || self.cred_i.is_some() {
            return Err(EDHOCError::IdentityAlreadySet);
        }
        self.i = Some(i);
        self.cred_i = Some(cred_i);
        Ok(())
    }

    pub fn verify_message_2(
        mut self,
        valid_cred_r: Credential,
    ) -> Result<EdhocInitiatorProcessedM2<Crypto>, EDHOCError> {
        trace!("Enter verify_message_2");
        let Some(i) = self.i else {
            return Err(EDHOCError::MissingIdentity);
        };
        match i_verify_message_2(&self.state, &mut self.crypto, valid_cred_r, &i) {
            Ok(state) => Ok(EdhocInitiatorProcessedM2 {
                state,
                cred_i: self.cred_i,
                crypto: self.crypto,
            }),
            Err(error) => Err(error),
        }
    }
}

impl<'a, Crypto: CryptoTrait> EdhocInitiatorProcessedM2<Crypto> {
    pub fn prepare_message_3(
        mut self,
        cred_transfer: CredentialTransfer,
        ead_3: &Option<EADItem>,
    ) -> Result<
        (
            EdhocInitiatorWaitM4<Crypto>,
            BufferMessage3,
            [u8; SHA256_DIGEST_LEN],
        ),
        EDHOCError,
    > {
        trace!("Enter prepare_message_3");
        let Some(cred_i) = self.cred_i else {
            return Err(EDHOCError::MissingIdentity);
        };
        match i_prepare_message_3(
            &mut self.state,
            &mut self.crypto,
            cred_i,
            cred_transfer,
            ead_3,
        ) {
            Ok((state, message_3, prk_out)) => Ok((
                EdhocInitiatorWaitM4 {
                    state,
                    crypto: self.crypto,
                },
                message_3,
                prk_out,
            )),
            Err(error) => Err(error),
        }
    }
}

impl<'a, Crypto: CryptoTrait> EdhocInitiatorWaitM4<Crypto> {
    pub fn process_message_4(
        mut self,
        message_4: &'a BufferMessage4,
    ) -> Result<(EdhocInitiatorDone<Crypto>, Option<EADItem>), EDHOCError> {
        trace!("Enter parse_message_4");
        match i_process_message_4(&mut self.state, &mut self.crypto, message_4) {
            Ok((state, ead_4)) => Ok((
                EdhocInitiatorDone {
                    state: state,
                    crypto: self.crypto,
                },
                ead_4,
            )),
            Err(error) => Err(error),
        }
    }

    pub fn completed_without_message_4(self) -> Result<EdhocResponderDone<Crypto>, EDHOCError> {
        trace!("Enter completed");
        match i_complete_without_message_4(&self.state) {
            Ok(state) => Ok(EdhocResponderDone {
                state,
                crypto: self.crypto,
            }),
            Err(error) => Err(error),
        }
    }
}

impl<Crypto: CryptoTrait> EdhocInitiatorDone<Crypto> {
    pub fn edhoc_exporter(
        &mut self,
        label: u8,
        context: &[u8],
        length: usize,
    ) -> [u8; MAX_BUFFER_LEN] {
        let mut context_buf: BytesMaxContextBuffer = [0x00u8; MAX_KDF_CONTEXT_LEN];
        context_buf[..context.len()].copy_from_slice(context);

        edhoc_exporter(
            &self.state,
            &mut self.crypto,
            label,
            &context_buf,
            context.len(),
            length,
        )
    }

    pub fn edhoc_key_update(&mut self, context: &[u8]) -> [u8; SHA256_DIGEST_LEN] {
        let mut context_buf = [0x00u8; MAX_KDF_CONTEXT_LEN];
        context_buf[..context.len()].copy_from_slice(context);

        edhoc_key_update(
            &mut self.state,
            &mut self.crypto,
            &context_buf,
            context.len(),
        )
    }
}

pub fn generate_connection_identifier_cbor<Crypto: CryptoTrait>(crypto: &mut Crypto) -> ConnId {
    let c_i = generate_connection_identifier(crypto);
    ConnId::from_int_raw(if c_i >= 0 && c_i <= 23 {
        c_i as u8 // verbatim encoding of single byte integer
    } else if c_i < 0 && c_i >= -24 {
        // negative single byte integer encoding
        CBOR_NEG_INT_1BYTE_START - 1 + c_i.unsigned_abs()
    } else {
        0
    })
}

/// generates an identifier that can be serialized as a single CBOR integer, i.e. -24 <= x <= 23
pub fn generate_connection_identifier<Crypto: CryptoTrait>(crypto: &mut Crypto) -> i8 {
    let mut conn_id = crypto.get_random_byte() as i8;
    while conn_id < -24 || conn_id > 23 {
        conn_id = crypto.get_random_byte() as i8;
    }
    conn_id
}

// Implements auth credential checking according to draft-tiloca-lake-implem-cons
pub fn credential_check_or_fetch(
    cred_expected: Option<Credential>,
    id_cred_received: IdCred,
) -> Result<Credential, EDHOCError> {
    trace!("Enter credential_check_or_fetch");
    // Processing of auth credentials according to draft-tiloca-lake-implem-cons
    // Comments tagged with a number refer to steps in Section 4.3.1. of draft-tiloca-lake-implem-cons
    if let Some(cred_expected) = cred_expected {
        // 1. Does ID_CRED_X point to a stored authentication credential? YES
        // IMPL: compare cred_i_expected with id_cred
        //   IMPL: assume cred_i_expected is well formed
        let credentials_match = if id_cred_received.reference_only() {
            id_cred_received.as_full_value() == cred_expected.by_kid()?.as_full_value()
        } else {
            id_cred_received.as_full_value() == cred_expected.by_value()?.as_full_value()
        };

        // 2. Is this authentication credential still valid?
        // IMPL,TODO: check cred_r_expected is still valid

        // Continue by considering CRED_X as the authentication credential of the other peer.
        // IMPL: ready to proceed, including process ead_2

        if credentials_match {
            Ok(cred_expected)
        } else {
            Err(EDHOCError::UnexpectedCredential)
        }
    } else {
        // 1. Does ID_CRED_X point to a stored authentication credential? NO
        // IMPL: cred_i_expected provided by application is None
        //       id_cred must be a full credential
        // 3. Is the trust model Pre-knowledge-only? NO (hardcoded to NO for now)
        // 4. Is the trust model Pre-knowledge + TOFU? YES (hardcoded to YES for now)
        // 6. Validate CRED_X. Generally a CCS has to be validated only syntactically and semantically, unlike a certificate or a CWT.
        //    Is the validation successful?
        // 5. Is the authentication credential authorized for use in the context of this EDHOC session?
        // IMPL,TODO: we just skip this step for now
        // 7. Store CRED_X as valid and trusted.
        //   Pair it with consistent credential identifiers, for each supported type of credential identifier.

        if let Some(cred) = id_cred_received.get_ccs() {
            Ok(cred)
        } else {
            Err(EDHOCError::ParsingError)
        }
    }

    // 8. Is this authentication credential good to use in the context of this EDHOC session?
    // IMPL,TODO: we just skip this step for now
}
