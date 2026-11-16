void *malloc(int n); void free(void *p);
int main(void) {
    int ***arr = malloc(16);
    int **b = malloc(8);
    int *v = malloc(4);
    if (!arr || !b || !v) return 0;
    arr[0] = b;
    arr[1] = b;
    int ***pp = arr + 1;
    **pp = v;
    int *r = *b;
    free(v);
    free(b);
    free(arr);
    return r == 0;
}
