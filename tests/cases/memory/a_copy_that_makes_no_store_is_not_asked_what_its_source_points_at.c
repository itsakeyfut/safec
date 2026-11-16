// A copy whose destination is no place makes no store, so it is asked at its
// arguments; and like every copy it is not asked what its source points at:
// copying the bytes of a freed pointer uses nothing (C17 7.24.2.1), and a
// read of what was copied is asked where it is read. The null destination
// is the defect here, `SC0403` (#979).
//
// Mutation: drop the return for a copy in `handed` before `freed_behind`.
// An `SC0407` at the copy comes back.
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void f(void) {
    int *x = malloc(4);
    if (!x) return;
    int **b = malloc(8);
    if (!b) return;
    *b = x;
    free(x);
    memcpy(0, b, 8);
}
