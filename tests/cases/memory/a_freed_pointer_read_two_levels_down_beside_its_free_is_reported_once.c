// `**t` is read beside a free of `t` that nothing orders against it, so the
// read is asked as a dereference and again as a read the free may follow:
// both are about the freed `t`, so one `SC0402` at `**t`, keyed by the
// place read through `t`'s own level (#987).
//
// Mutation: `Known::met` keys a dereference's pending read by the whole
// place again. A second `SC0402` appears at `**t`.
void free(void *p);
int g(int a);
int k6(int ***t) {
    return g(***t) + g(**t != 0) + (free(t), 0);
}
