void f(int * _Nonnull p) {
    int **pp = &p;
    *pp = 0;
    *p = 1;
}
