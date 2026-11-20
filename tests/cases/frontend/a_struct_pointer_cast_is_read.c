struct S {
    int a;
};
int f(void *v, struct S *s) {
    struct S *q = (struct S *)v;
    void *w = (struct S *)v + 1;
    return q == s && w != 0 && sizeof(struct S *) > 0;
}
