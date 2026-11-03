// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// golden/permits.json: the digests near-guardian signs (contracts/near-guardian/src/tests.rs writes
// them), recomputed by AntibodyAdapter at the same address and chain.

import {AntibodyAdapter} from "../../src/antibody/AntibodyAdapter.sol";

interface Vm {
    function readFile(string calldata) external view returns (string memory);
    function parseJsonUint(string calldata, string calldata) external pure returns (uint256);
    function parseJsonAddress(string calldata, string calldata) external pure returns (address);
    function parseJsonBytes32(string calldata, string calldata) external pure returns (bytes32);
    function parseJsonString(string calldata, string calldata) external pure returns (string memory);
    function chainId(uint256) external;
    function etch(address, bytes calldata) external;
}

contract GoldenPermitsTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_theAdapterComputesTheGuardiansDigests() public {
        string memory g = vm.readFile("../../golden/permits.json");
        address at = vm.parseJsonAddress(g, ".adapter");
        bytes32 program = keccak256(bytes(vm.parseJsonString(g, ".program_key")));
        vm.etch(at, address(new AntibodyAdapter(address(1), program, new address[](0), new address[](0))).code);
        AntibodyAdapter a = AntibodyAdapter(at);

        vm.chainId(vm.parseJsonUint(g, ".pause.chainId"));
        AntibodyAdapter.PausePermit memory p = AntibodyAdapter.PausePermit({
            program: program,
            round: uint32(vm.parseJsonUint(g, ".pause.round")),
            ticket: uint64(vm.parseJsonUint(g, ".pause.ticket")),
            deadline: vm.parseJsonUint(g, ".pause.deadline")
        });
        require(a.pauseDigest(p) == vm.parseJsonBytes32(g, ".pause.digest"), "pause digest");

        vm.chainId(vm.parseJsonUint(g, ".release.chainId"));
        AntibodyAdapter.ReleasePermit memory r = AntibodyAdapter.ReleasePermit({
            program: program,
            round: uint32(vm.parseJsonUint(g, ".release.round")),
            proxy: vm.parseJsonAddress(g, ".release.proxy"),
            codeHash: vm.parseJsonBytes32(g, ".release.codeHash"),
            deadline: vm.parseJsonUint(g, ".release.deadline")
        });
        require(a.releaseDigest(r) == vm.parseJsonBytes32(g, ".release.digest"), "release digest");
    }
}
