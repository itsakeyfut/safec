struct S {
    int a;
};
struct S make(void);
int take(struct S s);
int f(struct S *p, struct S *q) {
    struct S local;
    local = *p;
    *q = local;
    p = q;
    if (p == q) {
        return take(local);
    }
    local = make();
    return 0;
}
int main(void) {
    return 0;
}
