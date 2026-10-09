struct S;
int f(struct S *p) {
    return p->a;
}
struct S {
    int a;
};
int g(struct S *p) {
    return p->a;
}
