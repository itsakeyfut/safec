void free(void *p);

__attribute__((annotate("safec_unchecked")))
void h(int *p, int *q) {
#include "an_included_conclusion_is_dumped_where_its_include_is.h"
    free(q);
    free(q);
}
