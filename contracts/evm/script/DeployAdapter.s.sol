// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// Deploys a program's AntibodyAdapter with CREATE2 (the same address on every chain for the same
// arguments), then, as the Acme admin, makes it the guard's pauser and hands the AcmeLend proxy to it.
// From then on only permits signed by the program's guardian key can pause Acme or upgrade AcmeLend.
//
//   GUARDIAN_ADDRESS=0x.. PROGRAM_KEY=<near-antibody>/<id> ACME=deployments/<chain>.json \
//   forge script script/DeployAdapter.s.sol --rpc-url <url> --broadcast [--unlocked --sender <admin>]
// ADAPTER_OUT names the JSON file written (default deployments/adapter-<chainId>.json).

import {AntibodyAdapter, AntibodyProxy} from "../src/antibody/AntibodyAdapter.sol";
import {AcmeGuard} from "../src/acme/AcmeGuard.sol";

interface Vm {
    function envAddress(string calldata) external view returns (address);
    function envString(string calldata) external view returns (string memory);
    function envOr(string calldata, uint256) external view returns (uint256);
    function envOr(string calldata, string calldata) external view returns (string memory);
    function readFile(string calldata) external view returns (string memory);
    function parseJsonAddress(string calldata, string calldata) external pure returns (address);
    function startBroadcast() external;
    function startBroadcast(uint256) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata, string calldata, address) external returns (string memory);
    function serializeBytes32(string calldata, string calldata, bytes32) external returns (string memory);
    function serializeUint(string calldata, string calldata, uint256) external returns (string memory);
    function serializeString(string calldata, string calldata, string calldata) external returns (string memory);
    function writeJson(string calldata, string calldata) external;
    function toString(uint256) external pure returns (string memory);
}

contract DeployAdapter {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function run() external {
        address guardian = vm.envAddress("GUARDIAN_ADDRESS");
        string memory programKey = vm.envString("PROGRAM_KEY");
        string memory acme = vm.readFile(vm.envString("ACME"));
        AcmeGuard guard = AcmeGuard(vm.parseJsonAddress(acme, ".AcmeGuard"));
        address lend = vm.parseJsonAddress(acme, ".AcmeLend");
        bytes32 program = keccak256(bytes(programKey));

        address[] memory targets = new address[](1);
        targets[0] = address(guard);
        address[] memory proxies = new address[](1);
        proxies[0] = lend;

        uint256 key = vm.envOr("DEPLOYER_KEY", uint256(0));
        if (key == 0) vm.startBroadcast();
        else vm.startBroadcast(key);
        AntibodyAdapter adapter = new AntibodyAdapter{salt: program}(guardian, program, targets, proxies);
        guard.setPauser(address(adapter), true);
        AntibodyProxy(payable(lend)).changeAdmin(address(adapter));
        vm.stopBroadcast();

        string memory o = "adapter";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeString(o, "programKey", programKey);
        vm.serializeBytes32(o, "program", program);
        vm.serializeAddress(o, "guardian", guardian);
        vm.serializeAddress(o, "pauseTarget", address(guard));
        vm.serializeAddress(o, "proxy", lend);
        string memory json = vm.serializeAddress(o, "adapter", address(adapter));
        string memory def = string.concat("deployments/adapter-", vm.toString(block.chainid), ".json");
        vm.writeJson(json, vm.envOr("ADAPTER_OUT", def));
    }
}
