void fill(int **s, int *v);
int main(void) {
    int x = 0;
    int *m[2];
    fill(m + 1, &x);
    int *q = m[0];
    if (q != 0) { *q = 1; }
    return 0;
}
