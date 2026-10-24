void *malloc(int n);
void grow(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    grow(&b);
    grow(&b);
    return 0;
}
