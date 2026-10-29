void fill(int **s);
int main(void) {
    int *m[2];
    fill(m);
    int *r = m[0];
    return r != 0;
}
