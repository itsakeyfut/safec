struct S { int m; };

int f(struct S *p) {
#include "a_parenthesised_included_operand_that_starts_a_node_is_not_refused.h"
    ->m = 1;
#include "a_parenthesised_included_operand_that_starts_a_node_is_not_refused.h"
    , 1;
#include "a_parenthesised_included_operand_that_starts_a_node_is_not_refused.h"
    = 0;
    return 0;
}
