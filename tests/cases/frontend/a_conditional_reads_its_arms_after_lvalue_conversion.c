struct S {
    int a;
};
int main(void) {
    int v;
    int *const p = &v;
    int *q = &v;
    const int *cp = &v;
    void *vp = &v;
    struct S s;
    s.a = 2;
    const struct S c = s;
    struct S t = 1 ? c : s;
    int *r = 1 ? p : q;
    int *u = 1 ? vp : cp;
    return t.a + *r;
}
