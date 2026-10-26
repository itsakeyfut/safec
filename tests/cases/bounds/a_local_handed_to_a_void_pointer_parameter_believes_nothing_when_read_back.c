void keep(void *x);

int main(void) {
    int x = 0;
    int *v = &x;
    keep(&v);
    int **pi = &v;
    int *q = *pi;
    if (q != 0) {
        *q = 1;
    }
    return x;
}
