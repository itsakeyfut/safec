void g();
void h(void) {
    g();
}
void g(int * _Nonnull p) {
    *p = 1;
}
