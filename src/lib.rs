#![no_std]
extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use borsh::BorshDeserialize;
use borsh::BorshSerialize;
use solana_program::{
    account_info::{AccountInfo, next_account_info},
    clock, entrypoint,
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

entrypoint!(process_instruction);

pub const FEE_RECEIVER: Pubkey = Pubkey::new_from_array([
    183, 231, 26, 4, 170, 254, 122, 189, 151, 227, 199, 150, 219, 140, 137, 241, 208, 247, 231,
    185, 96, 41, 98, 183, 121, 165, 132, 99, 187, 65, 128, 48,
]);

pub const AUTHORITY: Pubkey = Pubkey::new_from_array([
    115, 70, 176, 17, 40, 35, 186, 108, 103, 93, 119, 77, 253, 9, 55, 46, 172, 41, 201, 158, 104,
    244, 46, 182, 56, 25, 197, 36, 89, 84, 13, 104,
]);

pub const MAGIC_BYTE: u8 = 0xAB;
pub const DATA_VERSION: u8 = 1;

#[derive(Debug)]
pub enum TokenInfoError {
    InvalidInstruction,
    AccountAlreadyExists,
    InsufficientFunds,
    InvalidLinkData,
}

impl From<TokenInfoError> for ProgramError {
    fn from(e: TokenInfoError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Images {
    pub icon: String,
    pub header: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct Link {
    pub label: String,
    pub url: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct TokenInfoV1 {
    pub mint: String,
    pub description: String,
    pub links: Vec<Link>,
    pub images: Images,
    pub creation_timestamp: i64,
    pub update_timestamp: i64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum TokenInfo {
    V1(TokenInfoV1),
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum Instruction {
    CreateInfo {
        description: String,
        links: Vec<Link>,
        icon_uri: String,
        header_uri: String,
    },
}

pub fn find_info_account(mint: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"token_info", mint.as_ref()], program_id)
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = Instruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    match instruction {
        Instruction::CreateInfo {
            description,
            links,
            icon_uri,
            header_uri,
        } => process_create_info(
            program_id,
            accounts,
            description,
            links,
            icon_uri,
            header_uri,
        ),
    }
}

fn process_create_info(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    description: String,
    links: Vec<Link>,
    icon_uri: String,
    header_uri: String,
) -> ProgramResult {
    msg!("[CreateInfo] Starting token info creation (V1)");

    let accounts_iter: &mut core::slice::Iter<'_, AccountInfo<'_>> = &mut accounts.iter();
    let payer_account = next_account_info(accounts_iter)?;
    let authority_account: &AccountInfo<'_> = next_account_info(accounts_iter)?;
    let mint_account = next_account_info(accounts_iter)?;
    let info_account = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let fee_receiver = next_account_info(accounts_iter)?;

    msg!("[CreateInfo] Validating signer and authority");
    if !payer_account.is_signer {
        msg!("[Error] Payer is not signer");
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !authority_account.is_signer {
        msg!("[Error] Authority is not signer");
        return Err(ProgramError::MissingRequiredSignature);
    }

    if authority_account.key != &AUTHORITY {
        msg!(
            "[Error] Invalid authority account: {:?}",
            authority_account.key
        );
        return Err(ProgramError::InvalidArgument);
    }

    if fee_receiver.key != &FEE_RECEIVER {
        msg!("[Error] Invalid fee receiver: {:?}", fee_receiver.key);
        return Err(ProgramError::InvalidArgument);
    }

    let fee_amount = 100_000_000;
    msg!("[CreateInfo] Checking payer balance >= {}", fee_amount);
    if payer_account.lamports() < fee_amount {
        msg!(
            "[Error] Insufficient funds: has {}, needs {}",
            payer_account.lamports(),
            fee_amount
        );
        return Err(TokenInfoError::InsufficientFunds.into());
    }

    msg!("[CreateInfo] Transferring fee to receiver");
    invoke(
        &system_instruction::transfer(payer_account.key, fee_receiver.key, fee_amount),
        &[
            payer_account.clone(),
            fee_receiver.clone(),
            system_program.clone(),
        ],
    )?;

    let (expected_info_address, bump_seed) = find_info_account(mint_account.key, program_id);
    msg!(
        "[CreateInfo] Derived info account: {:?}, bump: {}",
        expected_info_address,
        bump_seed
    );

    if expected_info_address != *info_account.key {
        msg!(
            "[Error] Info account mismatch. Expected: {:?}, got: {:?}",
            expected_info_address,
            info_account.key
        );
        return Err(ProgramError::InvalidArgument);
    }

    if !info_account.data_is_empty() {
        msg!("[Error] Info account already initialized");
        return Err(TokenInfoError::AccountAlreadyExists.into());
    }

    if *info_account.owner != *system_program.key {
        msg!("[Error] Info account owner mismatch. Expected system program");
        return Err(ProgramError::InvalidAccountData);
    }

    let clock = clock::Clock::get()?;
    let ts = clock.unix_timestamp;
    msg!("[CreateInfo] Timestamp: {}", ts);

    for link in &links {
        msg!("[CreateInfo] Adding link: {} -> {}", link.label, link.url);
    }

    let images = Images {
        icon: icon_uri.clone(),
        header: header_uri.clone(),
    };

    let info_v1 = TokenInfoV1 {
        mint: mint_account.key.to_string(),
        description: description.clone(),
        links,
        images,
        creation_timestamp: ts,
        update_timestamp: ts,
    };

    let info: TokenInfo = TokenInfo::V1(info_v1);

    let mut serialized_data = Vec::with_capacity(1024);
    serialized_data.push(MAGIC_BYTE);
    serialized_data.push(DATA_VERSION);
    info.serialize(&mut serialized_data)?;

    let rent = Rent::get()?;
    let lamports = rent.minimum_balance(serialized_data.len());
    msg!(
        "[CreateInfo] Creating account with rent exemption: {} lamports",
        lamports
    );

    invoke_signed(
        &system_instruction::create_account(
            payer_account.key,
            info_account.key,
            lamports,
            serialized_data.len() as u64,
            program_id,
        ),
        &[
            payer_account.clone(),
            info_account.clone(),
            system_program.clone(),
        ],
        &[&[b"token_info", mint_account.key.as_ref(), &[bump_seed]]],
    )?;

    info_account
        .data
        .borrow_mut()
        .copy_from_slice(&serialized_data);
    msg!("[CreateInfo] Token info account created and data written successfully");

    Ok(())
}
