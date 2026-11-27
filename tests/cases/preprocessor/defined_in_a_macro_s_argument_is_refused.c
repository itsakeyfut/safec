#define X 1
#define F(a) (a+1)
#define G F
#define OPEN F(
#define CALL(m) m
#define NOT !
#define ALSO_OPEN OPEN
#define SELF SELF
#define F2(a) F(
#define VIA_CALL F2(0)

#if F(defined(X)) == 2
int a;
#endif

#if G (defined X) == 2
int b;
#endif

#if OPEN defined X) == 2
int c;
#endif

#if CALL(F)(defined X) == 2
int e;
#endif

#if ALSO_OPEN defined X) == 2
int h;
#endif

#if VIA_CALL defined X) == 2
int j;
#endif

#if defined G || F(defined X) == 2
int m;
#endif
