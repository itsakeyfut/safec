int * _Nonnull f(int * _Nonnull p) {
    return p;
}

int main(void) {
    int x = 0;
    int *q = f(&x);
    return *q;
}
