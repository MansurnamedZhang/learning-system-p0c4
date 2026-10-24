"""Lossless Task7 corrupt-audit transport tests; no Docker or native claim."""
import copy
import hashlib
import unittest

from scripts.p0c4_completion import attention as a
from scripts.test_p0c4_attention_assets import proof_fixture


def legacy_bytes(before, after):
    # This order is the original Rust NegativeEvidence struct serialization.
    return (
        b'{"audit_failed":false,"injected":"dump_freeze",'
        b'"refused":"dump_freeze","before":' + before
        + b',"after":' + after + b'}'
    )


def fixture(part=None):
    values = proof_fixture('attention-corrupt-dump')
    if part is None:
        return values
    objects = values[0]
    data = a._json(part)
    for side in ('before', 'after'):
        objects['full-attention-corrupt-' + side + '.json'] = copy.deepcopy(data)
    index = objects['full-attention-corrupt-audit-index.json']
    original = legacy_bytes(part, part)
    for side in ('before', 'after'):
        index[side].update(size=len(part), sha256=a.sha(part))
    index['legacy'].update(size=len(original), sha256=a.sha(original))
    proof = objects['full-attention-result.json']
    proof['negative'].update(
        before_sha256=a.sha(part), after_sha256=a.sha(part),
        original_negative_inventory_sha256=a.sha(original),
    )
    objects['full-import-result.json']['negative_evidence'].update(
        bytes=len(original), sha256=a.sha(original),
    )
    repin_full_import(objects)
    return values


def repin_full_import(objects):
    objects['full-attention-result.json']['original_full_import_proof_sha256'] = (
        a.sha(a.canonical(objects['full-import-result.json']))
    )


def validate(values):
    objects, plan, target, capture = values
    return a.validate_bundle(
        {name: a.canonical(value) for name, value in objects.items()},
        plan, target, capture,
    )


class AttentionCorruptSplitTests(unittest.TestCase):
    def test_complete_large_snapshots_reconstruct_legacy_larger_than_file_cap(self):
        part = a.canonical(dict(
            catalog={'deterministic_catalog': 'x' * (9 * 1024**2)},
            assets={'retained': ['full byte source']},
            attempts=[{'id': 'unchanged', 'attempt_count': 3}],
        ))
        original = legacy_bytes(part, part)
        self.assertLess(len(part), 16 * 1024**2)
        self.assertGreater(len(original), 16 * 1024**2)
        self.assertEqual(
            a._corrupt_legacy_digest(part, part),
            (len(original), hashlib.sha256(original).hexdigest()),
        )
        with self.assertRaisesRegex(a.AttentionError, 'ATTENTION_PROOF_BYTES_CAP'):
            a._json(original)
        values = fixture(part)
        proof = validate(values)
        self.assertEqual(
            proof['negative']['original_negative_inventory_sha256'], a.sha(original),
        )
        self.assertNotIn('full-negative-inventory.json', values[0])
        for name, cap, _ in a.fixed_proof_files('attention-corrupt-dump'):
            self.assertLessEqual(len(a.canonical(values[0][name])), cap)

    def test_exact_case_rosters_and_all_six_existing_semantics(self):
        for case in ('attention-full-chain', *a.EXPECTED_REJECTIONS):
            with self.subTest(case=case):
                self.assertEqual(validate(proof_fixture(case))['case'], case)
        rows = a.fixed_proof_files('attention-corrupt-dump')
        self.assertEqual({name for name, _, _ in rows}, {
            'full-attention-result.json', 'full-attention-inventory.json',
            'full-import-result.json', 'full-attention-corrupt-audit-index.json',
            'full-attention-corrupt-before.json', 'full-attention-corrupt-after.json',
        })
        self.assertEqual(dict((name, cap) for name, cap, _ in rows)[
            'full-attention-corrupt-audit-index.json'], 16384)

    def test_legacy_claims_and_original_reference_require_complete_byte_reconstruction(self):
        kinds = (
            'legacy-sha', 'legacy-size', 'legacy-encoding', 'legacy-path',
            'reference-sha', 'reference-size', 'reference-stage',
            'reference-unchanged', 'sorted-envelope',
        )
        for kind in kinds:
            values = fixture()
            objects = values[0]
            index = objects['full-attention-corrupt-audit-index.json']
            original = objects['full-import-result.json']
            ref = original['negative_evidence']
            if kind == 'legacy-sha':
                index['legacy']['sha256'] = 'f' * 64
            elif kind == 'legacy-size':
                index['legacy']['size'] += 1
            elif kind == 'legacy-encoding':
                index['legacy']['encoding'] = 'caller_prefix'
            elif kind == 'legacy-path':
                index['legacy']['path'] = 'secrets/admin.dsn'
            elif kind == 'reference-sha':
                ref['sha256'] = 'f' * 64
            elif kind == 'reference-size':
                ref['bytes'] += 1
            elif kind == 'reference-stage':
                ref['stage'] = 'other'
            elif kind == 'reference-unchanged':
                ref['unchanged'] = False
            else:
                part = objects['full-attention-corrupt-before.json']
                claimed = a.canonical(dict(
                    before=part, after=part, injected='dump_freeze',
                    refused='dump_freeze', audit_failed=False,
                ))
                index['legacy'].update(size=len(claimed), sha256=a.sha(claimed))
                ref.update(bytes=len(claimed), sha256=a.sha(claimed))
                objects['full-attention-result.json']['negative'][
                    'original_negative_inventory_sha256'] = a.sha(claimed)
            repin_full_import(objects)
            with self.subTest(kind=kind), self.assertRaises(a.AttentionError):
                validate(values)

    def test_missing_extra_cropped_changed_or_unpinned_parts_refuse(self):
        for kind in ('missing', 'extra', 'path', 'size-bool', 'part-sha',
                     'changed-after', 'cropped-catalog', 'bad-flags'):
            values = fixture()
            objects = values[0]
            index = objects['full-attention-corrupt-audit-index.json']
            if kind == 'missing':
                objects.pop('full-attention-corrupt-after.json')
            elif kind == 'extra':
                objects['full-negative-inventory.json'] = {}
            elif kind == 'path':
                index['after']['path'] = 'full-attention-corrupt-before.json'
            elif kind == 'size-bool':
                index['before']['size'] = True
            elif kind == 'part-sha':
                index['after']['sha256'] = 'f' * 64
            elif kind == 'changed-after':
                objects['full-attention-corrupt-after.json']['catalog']['changed'] = True
            elif kind == 'cropped-catalog':
                objects['full-attention-corrupt-before.json'].pop('catalog')
            else:
                index['audit_failed'] = True
            with self.subTest(kind=kind), self.assertRaises(a.AttentionError):
                validate(values)

    def test_index_unknown_fields_wrong_domain_and_oversized_payload_refuse(self):
        for kind in ('extra', 'case', 'oversize'):
            values = fixture()
            index = values[0]['full-attention-corrupt-audit-index.json']
            if kind == 'extra':
                index['prefix'] = 'caller supplied byte grammar'
            elif kind == 'case':
                index['case'] = 'attention-missing-asset'
            else:
                index['capability'] = 'x' * 16385
                self.assertGreater(len(a.canonical(index)), 16384)
            with self.subTest(kind=kind), self.assertRaises(a.AttentionError):
                validate(values)

    def test_original_json_and_split_part_caps_remain_16mib(self):
        limit = 16 * 1024**2
        at_limit = b'x' * limit
        size, digest = a._corrupt_legacy_digest(at_limit, b'x')
        self.assertEqual((size, digest), (
            len(legacy_bytes(at_limit, b'x')), a.sha(legacy_bytes(at_limit, b'x')),
        ))
        for side in ('before', 'after'):
            before, after = (at_limit + b'x', b'x') if side == 'before' else (b'x', at_limit + b'x')
            with self.subTest(side=side), self.assertRaisesRegex(
                a.AttentionError, 'ATTENTION_SPLIT_PART_BYTES_CAP',
            ):
                a._corrupt_legacy_digest(before, after)
        with self.assertRaisesRegex(a.AttentionError, 'ATTENTION_PROOF_BYTES_CAP'):
            a._json(at_limit + b'x')
        rows = {name: cap for name, cap, _ in a.fixed_proof_files('attention-corrupt-dump')}
        for side in ('before', 'after'):
            self.assertEqual(rows['full-attention-corrupt-' + side + '.json'], limit)


if __name__ == '__main__':
    unittest.main()
