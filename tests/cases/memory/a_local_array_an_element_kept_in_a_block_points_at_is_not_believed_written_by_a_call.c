void *malloc(int n);
void fill3(int ***p);
int f(int k) {
    int *arr[2];
    if (k < 0) return 0;
    if (k > 1) return 0;
    int ***box = malloc(8);
    if (!box) return 0;
    *box = &arr[k];
    fill3(box);
    return arr[0] != 0;
}
