"""Task 5: role input validation precedes any cluster mutation."""
import copy
import json
import unittest
from scripts.p0c4_completion import roles


def recipe():
    return dict(format_version=1, roles=[dict(name=name, login=name!='learning_auth_lock', inherit=True,
        superuser=False, createdb=False, createrole=False, bypassrls=False, replication=False,
        connection_limit=-1) for name in ('learning_admin','learning_auth_lock','learning_runtime')],
        memberships=[dict(role='learning_auth_lock',member='learning_admin',inherit=False,set=True,admin=False)])


class FullRestoreRoles(unittest.TestCase):
    def test_validated_recipe_preserves_flags_and_limits(self):
        value=recipe();value['roles'][0]['inherit']=False;value['roles'][2]['connection_limit']=5
        result=roles._validate_recipe(json.dumps(value,separators=(',',':')).encode())
        self.assertEqual(result,value)

    def test_unsafe_roles_and_membership_refuse_before_provisioning(self):
        for change in ('createrole','superuser','createdb','replication','bypassrls'):
            value=recipe();value['roles'][0][change]=True
            with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(json.dumps(value).encode())
        for field,value in [('inherit',True),('set',False),('admin',True)]:
            bad=recipe();bad['memberships'][0][field]=value
            with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(json.dumps(bad).encode())

    def test_unknown_fields_duplicate_keys_and_non_boolean_flags_refuse(self):
        for change in ('password','sql','roleconfig'):
            bad=recipe();bad['roles'][0][change]='untrusted'
            with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(json.dumps(bad).encode())
        bad=recipe();bad['roles'][0]['login']=1
        with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(json.dumps(bad).encode())
        raw=json.dumps(recipe()).replace('"format_version": 1','"format_version": 1, "format_version": 1')
        with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(raw.encode())

    def test_invalid_role_count_order_limit_and_oversize_refuse(self):
        for mutation in ('missing','order','limit','type'):
            bad=copy.deepcopy(recipe())
            if mutation=='missing':bad['roles'].pop()
            elif mutation=='order':bad['roles'].reverse()
            elif mutation=='limit':bad['roles'][0]['connection_limit']=-2
            else:bad['roles'][0]['connection_limit']=True
            with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(json.dumps(bad).encode())
        with self.assertRaises(roles.RoleProvisioningError):roles._validate_recipe(b' '*16385)

    def test_plain_values_cannot_construct_provisioning_authority(self):
        for factory,args in [(roles.VerifiedRoleRecipe,(None,json.dumps(recipe()).encode(),{})),(roles.FreshTargetPlan,(None,{},None,None)),(roles.RoleProvisioningReceipt,(None,{},b'{}',''))]:
            with self.assertRaises(roles.RoleProvisioningError):factory(*args)
        with self.assertRaises(roles.RoleProvisioningError):roles.provision_full_restore_roles(recipe(),{})


if __name__=='__main__':unittest.main()
