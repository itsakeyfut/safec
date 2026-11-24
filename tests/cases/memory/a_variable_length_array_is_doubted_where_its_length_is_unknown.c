int first(int n) {
    int a[n];
    a[0] = 1;
    return a[0];
}

int at_its_length(int n) {
    int a[n];
    a[n] = 1;
    return 0;
}

int main(void) {
    return first(3) + at_its_length(3);
}
