void *malloc(int n);

int main(void) {
    int *p = malloc(16);
    if (p == 0) {
        return 1;
    }
    int *v = p + 4;
    int **pi = &v;
    **pi = 1;
    return 0;
}
