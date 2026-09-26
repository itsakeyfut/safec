void g(int * _Nonnull p);
int f(int **pp) {
    g(*pp);
    return 0;
}
