struct S { int x; char *p; };
struct S;
struct T s;
int f(struct S *p) {
    struct S q;
    return p->x + q.x;
}
int g(void) {
    int k = 1;
    return k;
}
int h(struct S *a, struct S *b) {
    a = b;
    return 0;
}
