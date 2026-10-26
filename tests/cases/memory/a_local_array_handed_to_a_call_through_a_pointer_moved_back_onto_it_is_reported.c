void fill(int **p);

int main(void) {
    int *m[2];
    int **d = &m[1];
    d = d - 1;
    fill(d);
    int *q = m[0];
    return q == 0;
}
