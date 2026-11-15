// As `a_use_after_freeing_a_local_an_int_was_stored_past_through_a_moved_pointer_is_not_proved`,
// with the moved pointer one level above the write: `**ppi = n` lands where
// `pi` points, which was moved off `z`, so `z` is written or written past
// and no longer certainly holds `a`.
//
// Mutation: `Allocations::past_end_targets` answers nothing for a depth
// above one. `*a` becomes a certain `SC0402`, used after it was freed.
void *malloc(int n);
void free(void *p);

int f(void) {
    int *a = malloc(4);
    if (!a) return 0;
    int *z = a;
    void *v = &z;
    int *pi = v;
    pi = pi + 2;
    int n = 5;
    int **ppi = &pi;
    **ppi = n;
    free(z);
    return *a;
}
