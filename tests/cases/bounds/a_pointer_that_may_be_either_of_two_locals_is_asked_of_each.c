void *malloc(int n);

int main(int c) {
    int *p = malloc(16);
    if (p == 0) {
        return 1;
    }
    int *a = p;
    int *b = p;
    int **pi = &b;
    if (c) {
        pi = &a;
    }
    a = p + 4;
    **pi = 1;
    return 0;
}
