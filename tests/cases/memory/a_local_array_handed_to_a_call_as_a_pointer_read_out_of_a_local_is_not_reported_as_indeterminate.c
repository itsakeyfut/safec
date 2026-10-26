void fill(int **m);

int main(void) {
    int *m[1];
    int **p = m;
    int ***pp = &p;
    fill(*pp);
    int *q = m[0];
    return q == 0;
}
