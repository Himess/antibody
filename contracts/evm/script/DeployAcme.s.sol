// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// Deploys the fictional "Acme Lend" fixture with CREATE2 (same addresses and code hashes on every
// chain for the same deployer), AcmeLend and AcmeVault behind AntibodyProxies, optionally seeds it,
// registers the pauser, deploys the implementations of the round-1 fix and of the final fix, and
// writes deployments/<chainId>.json including the invariants' runtime bytecode. The invariants contract is only instantiated in the local
// simulation: it is never broadcast.
//
//   DEPLOYER_KEY=<env> GUARDIAN_ADDRESS=0x.. SEED=true forge script script/DeployAcme.s.sol --rpc-url <url> --broadcast
// Local chains can skip the key: `--unlocked --sender <anvil account>` (DEPLOYER_KEY unset).
// DEPLOYMENTS_OUT overrides the output file.

import {AcmeSetup} from "../src/acme/AcmeSetup.sol";
import {AcmeInvariants} from "../src/acme/AcmeInvariants.sol";
import {IAcmeToken} from "../src/acme/AcmeTokens.sol";
import {AcmeLendFixed} from "../src/acme/AcmeLendFixed.sol";
import {AcmeLendFinal, AcmeVaultFixed} from "../src/acme/AcmeFinalFix.sol";

interface Vm {
    function envOr(string calldata, address) external view returns (address);
    function envOr(string calldata, uint256) external view returns (uint256);
    function envOr(string calldata, string calldata) external view returns (string memory);
    function startBroadcast() external;
    function envOr(string calldata, bool) external view returns (bool);
    function addr(uint256) external pure returns (address);
    function startBroadcast(uint256) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata, string calldata, address) external returns (string memory);
    function serializeBytes32(string calldata, string calldata, bytes32) external returns (string memory);
    function serializeBytes(string calldata, string calldata, bytes calldata) external returns (string memory);
    function serializeUint(string calldata, string calldata, uint256) external returns (string memory);
    function serializeBool(string calldata, string calldata, bool) external returns (string memory);
    function writeJson(string calldata, string calldata) external;
    function toString(uint256) external pure returns (string memory);
}

contract DeployAcme {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function run() external {
        uint256 key = vm.envOr("DEPLOYER_KEY", uint256(0));
        address admin = key == 0 ? msg.sender : vm.addr(key);
        address guardian = vm.envOr("GUARDIAN_ADDRESS", address(0));
        bool seed = vm.envOr("SEED", true);

        if (key == 0) vm.startBroadcast();
        else vm.startBroadcast(key);
        AcmeSetup.Deployment memory d = AcmeSetup.deploy(admin);
        if (guardian != address(0)) d.guard.setPauser(guardian, true);
        // The guardian address pays for its own pause() transactions.
        uint256 fund = vm.envOr("GUARDIAN_FUND_WEI", uint256(0));
        if (guardian != address(0) && fund > 0) payable(guardian).transfer(fund);
        if (seed) AcmeSetup.seed(d, admin);
        // The project's round-1 fix, deployed where a passed release gate upgrades the proxy to. A round
        // judges it as a code override at the implementation's address (AcmeLendImpl).
        AcmeLendFixed fixedLend = new AcmeLendFixed{salt: AcmeSetup.SALT}(
            d.guard, IAcmeToken(address(d.usd)), IAcmeToken(address(d.eth)), d.pool, AcmeSetup.P_REF
        );
        // The final fix (round 2): both implementations a passed release gate upgrades the proxies to.
        AcmeLendFinal finalLend = new AcmeLendFinal{salt: AcmeSetup.SALT}(
            d.guard, IAcmeToken(address(d.usd)), IAcmeToken(address(d.eth)), d.pool, AcmeSetup.P_REF
        );
        AcmeVaultFixed fixedVault =
            new AcmeVaultFixed{salt: AcmeSetup.SALT}(d.guard, IAcmeToken(address(d.usd)), d.lend, address(d.treasury), admin);
        vm.stopBroadcast();

        // Local simulation only: the runtime bytecode of the invariants the program commits to.
        AcmeInvariants inv =
            new AcmeInvariants(d.lend, d.vault, IAcmeToken(address(d.usd)), address(d.treasury), AcmeSetup.P_REF);

        string memory o = "deployment";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeAddress(o, "admin", admin);
        vm.serializeAddress(o, "guardian", guardian);
        vm.serializeBool(o, "seeded", seed);
        vm.serializeAddress(o, "AcmeGuard", address(d.guard));
        vm.serializeAddress(o, "acUSD", address(d.usd));
        vm.serializeAddress(o, "acETH", address(d.eth));
        vm.serializeAddress(o, "AcmePool", address(d.pool));
        vm.serializeAddress(o, "AcmeLend", address(d.lend));
        vm.serializeAddress(o, "AcmeLendImpl", address(d.lendImpl));
        vm.serializeAddress(o, "AcmeLendFixed", address(fixedLend));
        vm.serializeAddress(o, "AcmeTreasury", address(d.treasury));
        vm.serializeAddress(o, "AcmeVault", address(d.vault));
        vm.serializeAddress(o, "AcmeVaultImpl", address(d.vaultImpl));
        vm.serializeAddress(o, "AcmeLendFinal", address(finalLend));
        vm.serializeAddress(o, "AcmeVaultFixed", address(fixedVault));
        vm.serializeAddress(o, "borrower1", address(d.borrower1));
        vm.serializeAddress(o, "borrower2", address(d.borrower2));
        vm.serializeUint(o, "pRef", AcmeSetup.P_REF);
        vm.serializeBytes32(o, "codehashAcmeLend", address(d.lend).codehash);
        vm.serializeBytes32(o, "codehashAcmeLendImpl", address(d.lendImpl).codehash);
        vm.serializeBytes32(o, "codehashAcmeVault", address(d.vault).codehash);
        vm.serializeBytes32(o, "codehashAcmeVaultImpl", address(d.vaultImpl).codehash);
        vm.serializeBytes32(o, "codehashAcmeLendFinal", address(finalLend).codehash);
        vm.serializeBytes32(o, "codehashAcmeVaultFixed", address(fixedVault).codehash);
        vm.serializeBytes(o, "finalLendRuntimeCode", address(finalLend).code);
        vm.serializeBytes(o, "fixedVaultRuntimeCode", address(fixedVault).code);
        vm.serializeBytes32(o, "codehashAcmePool", address(d.pool).codehash);
        vm.serializeBytes32(o, "codehashAcmeGuard", address(d.guard).codehash);
        vm.serializeBytes32(o, "invariantsCodeHash", address(inv).codehash);
        vm.serializeBytes32(o, "codehashAcmeLendFixed", address(fixedLend).codehash);
        vm.serializeBytes(o, "fixedLendRuntimeCode", address(fixedLend).code);
        string memory json = vm.serializeBytes(o, "invariantsRuntimeCode", address(inv).code);
        string memory def = string.concat("deployments/", vm.toString(block.chainid), ".json");
        vm.writeJson(json, vm.envOr("DEPLOYMENTS_OUT", def));
    }
}
