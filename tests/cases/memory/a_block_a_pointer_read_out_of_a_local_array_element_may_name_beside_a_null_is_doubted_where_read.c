void *malloc(int n);
void fill(int **p);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int **arr[1];
    arr[0] = 0;
    if (c) arr[0] = b;
    int **p = arr[0];
    fill(p);
    return *b != 0;
}
