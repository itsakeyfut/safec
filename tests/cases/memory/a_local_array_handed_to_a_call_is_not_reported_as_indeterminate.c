void fill(int **m);

int main(void) {
    int *m[1];
    fill(m);
    int *q = m[0];
    return q == 0;
}
