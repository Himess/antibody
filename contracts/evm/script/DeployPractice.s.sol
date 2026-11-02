// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET — never deploy to mainnet. Deploys the practice program's vaults (docs/practice.md):
// a factory that funds COUNT vaults with VALUE_WEI in total, and writes deployments/practice-<chainId>.json
// with every vault's address and the invariants' runtime bytecode (instantiated in the local simulation
// only, never broadcast).
//
//   COUNT=24 VALUE_WEI=2400000000000000 DEPLOYER_KEY=<env> forge script script/DeployPractice.s.sol --rpc-url <url> --broadcast
// Local chains can skip the key: `--unlocked --sender <anvil account>` (DEPLOYER_KEY unset).

import {PracticeFactory, PracticeInvariants} from "../src/practice/Practice.sol";

interface Vm {
    function envOr(string calldata, uint256) external view returns (uint256);
    function envOr(string calldata, string calldata) external view returns (string memory);
    function startBroadcast() external;
    function startBroadcast(uint256) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata, string calldata, address) external returns (string memory);
    function serializeAddress(string calldata, string calldata, address[] calldata) external returns (string memory);
    function serializeBytes32(string calldata, string calldata, bytes32) external returns (string memory);
    function serializeBytes(string calldata, string calldata, bytes calldata) external returns (string memory);
    function serializeUint(string calldata, string calldata, uint256) external returns (string memory);
    function writeJson(string calldata, string calldata) external;
    function toString(uint256) external pure returns (string memory);
}

contract DeployPractice {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function run() external {
        uint256 count = vm.envOr("COUNT", uint256(24));
        uint256 value = vm.envOr("VALUE_WEI", uint256(2_400_000_000_000_000));
        uint256 key = vm.envOr("DEPLOYER_KEY", uint256(0));
        if (key == 0) vm.startBroadcast();
        else vm.startBroadcast(key);
        PracticeFactory factory = new PracticeFactory{value: value}(count);
        vm.stopBroadcast();

        // Local simulation only: the runtime bytecode of the rule the program commits to.
        PracticeInvariants inv = new PracticeInvariants(factory);
        address[] memory vaults = new address[](count);
        for (uint256 i; i < count; ++i) {
            vaults[i] = factory.vaultAt(i);
        }

        string memory o = "practice";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeAddress(o, "factory", address(factory));
        vm.serializeBytes32(o, "codehashFactory", address(factory).codehash);
        vm.serializeBytes32(o, "codehashVault", vaults[0].codehash);
        vm.serializeAddress(o, "vaults", vaults);
        vm.serializeBytes32(o, "invariantsCodeHash", address(inv).codehash);
        string memory json = vm.serializeBytes(o, "invariantsRuntimeCode", address(inv).code);
        string memory def = string.concat("deployments/practice-", vm.toString(block.chainid), ".json");
        vm.writeJson(json, vm.envOr("PRACTICE_OUT", def));
    }
}
