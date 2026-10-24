int *pick(int * _Nonnull p) {
    return p;
}

int main(void) {
    int x = 0;
    int *q = pick(&x);
    return *q;
}
