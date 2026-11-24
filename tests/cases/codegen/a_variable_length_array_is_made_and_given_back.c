int sized(int n) {
    int size = 0;
    {
        char a[n];
        a[0] = 1;
        size = a[0] + (int)sizeof(a);
    }
    return size;
}
