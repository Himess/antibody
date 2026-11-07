// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET: the practice program (docs/practice.md in the Antibody repository). Written by
// `antibody hunt start` for {{ACCOUNT}}: its instance is vault-{{INSTANCE}}, {{VAULT}} at the program's
// anchor.
//
// `forge test` deploys the same instance here and needs no RPC. Forked at the anchor (`--fork-url <Sepolia
// RPC> --fork-block-number {{ANCHOR_BLOCK}}`) it uses the program's own contracts instead.
//
// test_proof holds the tutorial proof as an example. Put your own calls there, each made as SENDER, then
// write them as a bundle (PROOF.md) for `antibody check` and `antibody hunt submit`: the bundle's calls go to
// the anchor's addresses (instance.json), which differ from this local deployment's.

import {PracticeFactory, PracticeVault, PracticeInvariants} from "../src/practice/Practice.sol";

interface Vm {
    function prank(address, address) external;
    function deal(address, uint256) external;
}

contract InstanceTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    /// Your instance: the vault index the vending contract assigned to {{ACCOUNT}}.
    uint256 constant INSTANCE = {{INSTANCE}};
    /// The program's factory and your vault at its anchor.
    address constant ANCHOR_FACTORY = {{FACTORY}};
    address constant ANCHOR_VAULT = {{VAULT}};
    /// As at the anchor: {{VAULTS}} vaults, 0.001 ether each.
    uint256 constant VAULTS = {{VAULTS}};
    uint256 constant EACH = 0.001 ether;
    /// Your bundle's sender 0. The verifier derives a fresh one from a secret salt, empty at the anchor.
    address constant SENDER = address(0xA0A0);
    /// The bundle's endowment: the ether its sender starts with (its gas is paid from it).
    uint256 constant ENDOWMENT = 0.001 ether;

    PracticeFactory factory;
    PracticeInvariants rules;
    PracticeVault vault;
    bytes pre;

    function setUp() public {
        factory = ANCHOR_VAULT.code.length > 0 ? PracticeFactory(ANCHOR_FACTORY) : new PracticeFactory{value: VAULTS * EACH}(VAULTS);
        rules = new PracticeInvariants(factory);
        vault = PracticeVault(payable(factory.vaultAt(INSTANCE)));
        // The verifier requires the rules to hold at the anchor, before any proof.
        pre = rules.snapshot();
        require(rules.check(pre) == 0, "the rule holds before the proof");
    }

    /// Your proof: the bundle's calls, in order, each as SENDER. Here, the tutorial proof (PROOF.md).
    function test_proof() public {
        vm.deal(SENDER, ENDOWMENT);
        vm.prank(SENDER, SENDER);
        vault.sweep(payable(address(0xdEaD)));
        require(rules.check(pre) != 0, "no rule breaks: on chain this proof would be a NoViolation, its bond to the program's escrow");
    }
}
