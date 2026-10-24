struct S;
void f(struct S *p, struct S *q) {
    *p = *q;
}
int g(struct S *p) {
    return *p, 0;
}
int h(struct S *p) {
    return &*p != 0;
}
