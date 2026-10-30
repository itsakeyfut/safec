void fill(int **s);
int main(void) {
    int s = 0;
    for (int i = 0; i < 2; i++) {
        int *m[2];
        if (i == 0) {
            fill(m + 1);
        } else {
            fill(m);
            int *r = m[0];
            s = s + (r != 0);
        }
    }
    return s;
}
