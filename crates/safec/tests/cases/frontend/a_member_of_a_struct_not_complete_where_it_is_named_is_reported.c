struct S;
struct S *p;
int f(void) {
    return p->a;
}
struct S {
    int a;
};
int g(void) {
    return p->a;
}
