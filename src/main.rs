// Fetches all vote account and for each, fetches validator info.  Stores the details thereof.
use borsh::BorshDeserialize;
use solana_client::rpc_client::RpcClient;
use solana_ledger::leader_schedule::LeaderSchedule;
use solana_sdk::clock::NUM_CONSECUTIVE_LEADER_SLOTS;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::stake::state::StakeState;
use std::collections::HashMap;

const DEFAULT_MAINNET_RPC_URL : &str = "https://api.mainnet-beta.solana.com";
const DEFAULT_TESTNET_RPC_URL : &str = "https://api.testnet.solana.com";
const DEFAULT_DEVNET_RPC_URL : &str = "https://api.devnet.solana.com";
const DEFAULT_LOCALHOST_RPC_URL : &str = "http://localhost:8899";

const SLOTS_IN_EPOCH : u64 = 432000;

struct Args
{
    url : String
}

fn error_exit(msg : String) -> !
{
    eprintln!("{}", msg);
    std::process::exit(-1);
}

fn parse_args() -> Result<Args, String>
{
    let mut args = std::env::args();

    args.nth(0);

    let mut url = None;

    while let Some(arg) = args.nth(0) {
        match arg.as_str() {
            "-u" | "--url" => {
                if url.is_some() {
                    eprintln!("ERROR: Duplicate {} argument", arg);
                    std::process::exit(-1);
                }
                url = match args.nth(0) {
                    None => error_exit(format!("ERROR: {} requires an argument", arg)),
                    Some(arg) => Some(arg.clone())
                };
            },
            _ => error_exit(format!("ERROR: Unexpected extra argument {}", arg))
        }
    }

    Ok(Args { url : get_url(url) })
}

fn get_url(url : Option<String>) -> String
{
    url.map_or_else(
        || DEFAULT_MAINNET_RPC_URL.to_string(),
        |url| match url.as_str() {
            "l" | "localhost" => DEFAULT_LOCALHOST_RPC_URL.to_string(),
            "d" | "devnet" => DEFAULT_DEVNET_RPC_URL.to_string(),
            "t" | "testnet" => DEFAULT_TESTNET_RPC_URL.to_string(),
            "m" | "mainnet" => DEFAULT_MAINNET_RPC_URL.to_string(),
            _ => url.clone()
        }
    )
}

fn main()
{
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(-1);
    });

    let rpc_client = RpcClient::new_with_commitment(args.url, CommitmentConfig::finalized());

    // Fetch current epoch
    let current_epoch = rpc_client
        .get_epoch_info()
        .unwrap_or_else(|e| error_exit(format!("ERROR: Failed to fetch epoch info: {}", e.to_string())))
        .epoch;

    // Fetch stakes in current epoch
    let response = rpc_client
        .get_program_accounts(&solana_sdk::stake::program::id())
        .unwrap_or_else(|e| error_exit(format!("ERROR: Failed to fetch stake accounts: {}", e.to_string())));

    let mut stakes = HashMap::<Pubkey, u64>::new();

    for (pubkey, account) in response {
        // Zero-length accounts owned by the stake program are system accounts that were re-assigned and are to be
        // ignored
        if account.data.len() == 0 {
            continue;
        }

        match StakeState::deserialize(&mut account.data.as_slice())
            .unwrap_or_else(|e| error_exit(format!("Failed to decode stake account {}: {}", pubkey, e)))
        {
            StakeState::Stake(_, stake) => {
                // Ignore stake accounts activated in this epoch (or later, to include activation_epoch of
                // u64::MAX which indicates no activation ever happened)
                if stake.delegation.activation_epoch >= current_epoch {
                    continue;
                }
                // Ignore stake accounts deactivated before this epoch
                if stake.delegation.deactivation_epoch < current_epoch {
                    continue;
                }
                // Add the stake in this stake account to the total for the delegated-to vote account
                *(stakes.entry(stake.delegation.voter_pubkey.clone()).or_insert(0)) += stake.delegation.stake;
            },
            _ => ()
        }
    }

    println!("The leader schedule for {} will be:", (current_epoch + 1));

    for leader in leader_schedule(current_epoch + 1, stakes).get_slot_leaders() {
        println!("{}", leader);
    }
}

// Cribbed from leader_schedule_utils
fn sort_stakes(stakes : &mut Vec<(Pubkey, u64)>)
{
    // Sort first by stake. If stakes are the same, sort by pubkey to ensure a
    // deterministic result.
    // Note: Use unstable sort, because we dedup right after to remove the equal elements.
    stakes.sort_unstable_by(|(l_pubkey, l_stake), (r_pubkey, r_stake)| {
        if r_stake == l_stake {
            r_pubkey.cmp(l_pubkey)
        }
        else {
            r_stake.cmp(l_stake)
        }
    });

    // Now that it's sorted, we can do an O(n) dedup.
    stakes.dedup();
}

// Mostly cribbed from leader_schedule_utils
fn leader_schedule(
    epoch : u64,
    stakes : HashMap<Pubkey, u64>
) -> LeaderSchedule
{
    let mut seed = [0u8; 32];
    seed[0..8].copy_from_slice(&epoch.to_le_bytes());
    let mut stakes : Vec<_> = stakes.iter().map(|(pubkey, stake)| (*pubkey, *stake)).collect();
    sort_stakes(&mut stakes);
    LeaderSchedule::new(&stakes, seed, SLOTS_IN_EPOCH, NUM_CONSECUTIVE_LEADER_SLOTS)
}
