struct S {
    int a;
};
void v(void);
int f(struct S s, int x) {
    return (int)s + (int)v();
}
struct S g(int x) {
    return (struct S)x;
}
