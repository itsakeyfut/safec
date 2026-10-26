int pick(int a[][3], int i, char c) {
    int *row = a[i];
    return *(row + c) + *(row - 1 + c);
}
int main(void) {
    int m[2][3];
    m[1][1] = 4;
    m[1][2] = 5;
    return pick(m, 1, 2);
}
