int * _Nonnull f(void), *g(void);

int main(void) {
    int *q = g();
    return *q;
}
