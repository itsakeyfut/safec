#define X 1
#define F(a) (a+1)
#define G F
#define OPEN F(

#if F(defined(X)) == 2
int a;
#endif

#if G (defined X) == 2
int b;
#endif

#if OPEN defined X) == 2
int c;
#endif
