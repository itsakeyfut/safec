void *malloc(unsigned long long);
void free(void *);
#include "a_pragma_whose_name_goes_on_past_once_is_not_once.h"
#include "a_pragma_whose_name_goes_on_past_once_is_not_once.h"
int main(void) {
    char *b = malloc(SIZE);
    if (!b) return 0;
    b[1] = 0;
    free(b);
    return 0;
}
