// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// Deploys the fictional "Acme Lend" fixture with CREATE2 (same addresses and code hashes on every
// chain for the same deployer), optionally seeds it, registers the guardian as pauser, and writes
// deployments/<chainId>.json including the invariants' runtime bytecode. The invariants contract
// is only instantiated in the local simulation: it is never broadcast.
//
//   DEPLOYER_KEY=<env> GUARDIAN_ADDRESS=0x.. SEED=true forge script script/DeployAcme.s.sol --rpc-url <url> --broadcast

import {AcmeSetup} from "../src/acme/AcmeSetup.sol";
import {AcmeInvariants} from "../src/acme/AcmeInvariants.sol";
import {IAcmeToken} from "../src/acme/AcmeTokens.sol";

interface Vm {
    function envUint(string calldata) external view returns (uint256);
    function envOr(string calldata, address) external view returns (address);
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
        uint256 key = vm.envUint("DEPLOYER_KEY");
        address admin = vm.addr(key);
        address guardian = vm.envOr("GUARDIAN_ADDRESS", address(0));
        bool seed = vm.envOr("SEED", true);

        vm.startBroadcast(key);
        AcmeSetup.Deployment memory d = AcmeSetup.deploy(admin);
        if (guardian != address(0)) d.guard.setPauser(guardian, true);
        if (seed) AcmeSetup.seed(d, admin);
        vm.stopBroadcast();

        // Local simulation only: compute the runtime bytecode the program commits to.
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
        vm.serializeAddress(o, "AcmeTreasury", address(d.treasury));
        vm.serializeAddress(o, "AcmeVault", address(d.vault));
        vm.serializeAddress(o, "borrower1", address(d.borrower1));
        vm.serializeAddress(o, "borrower2", address(d.borrower2));
        vm.serializeUint(o, "pRef", AcmeSetup.P_REF);
        vm.serializeBytes32(o, "codehashAcmeLend", address(d.lend).codehash);
        vm.serializeBytes32(o, "codehashAcmeVault", address(d.vault).codehash);
        vm.serializeBytes32(o, "codehashAcmePool", address(d.pool).codehash);
        vm.serializeBytes32(o, "codehashAcmeGuard", address(d.guard).codehash);
        vm.serializeBytes32(o, "invariantsCodeHash", address(inv).codehash);
        string memory json = vm.serializeBytes(o, "invariantsRuntimeCode", address(inv).code);
        vm.writeJson(json, string.concat("deployments/", vm.toString(block.chainid), ".json"));
    }
}
